// #![deny(warnings)]
#![allow(incomplete_features)]
#![feature(unsized_locals, unsized_fn_params)]
#![feature(get_mut_unchecked)]
#![feature(type_ascription)]

extern crate futures;
extern crate tokio_core;

use std::collections::HashMap;
use std::sync::{
    Arc
};
use std::convert::Infallible;
use futures::{StreamExt};
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::ws::{Message, WebSocket};
use warp::Filter;
use serde_json::{Value, Map};

use std::str;

use std::env;
use std::net::{SocketAddr};
use redis_async::{client};
use redis_async::client::{PubsubConnection};

use dashmap::DashMap;

mod intos;
mod structs;

use snowflake::ProcessUniqueId;

use crate::structs::{Subscription, SubscriptionTracker};
use crate::intos::*;

#[macro_use] extern crate lazy_static;

type Clients = Arc<DashMap<String, mpsc::UnboundedSender<Result<Message, warp::Error>>>>;
type ManagedSubscriptionTracker = Arc<SubscriptionTracker>;

lazy_static! {
    static ref PONG_MESSAGE: Message = Message::text("{\"o\":1}");
    static ref OK_MESSAGE: Message = Message::text("{\"o\":5}");
}

fn insert_sender(sender: mpsc::UnboundedSender<Result<Message, warp::Error>>, clients: Clients) -> String {
    let client_id = format!("{}", ProcessUniqueId::new()).to_string();
    let copied = client_id.clone();

    clients.insert(client_id, sender);

    return copied;
}

macro_rules! handle_send {
    ($result:expr) => {{
        match $result {
            Err(_) => return,
            _ => return
        }
    }}
}

macro_rules! unwrap_option_or_break {
    ($result:expr) => {{
        match $result {
            Some(res) => res,
            _ => break
        }
    }}
}

async fn send_message(message: Message, id: String, clients: Clients) {
    let client = match clients.get(&id) {
        Some(client_ref) => client_ref,
        _ => return
    };

    handle_send!(client.send(Ok(message)));
}

async fn redis_event_track_attempt(subscribed: bool, stream: String, payload: Map<String, Value>, id: String, subscription_tracker: ManagedSubscriptionTracker) {
    let keypaths = match payload["d"].into_string_vec() {
        Some(raw_paths) => raw_paths,
        _ => return
    };

    subscription_tracker.set_subscribed(subscribed, keypaths, stream, id);
}

fn make_update_packet(keypath: String, data: String) -> String {
    format!("{{\"o\":4,\"p\":\"{}\",\"d\":{}}}", keypath, data)
}

fn make_bulk_update_packet(packets: Vec<String>) -> String {
    format!("{{\"o\":6,\"d\":[{}]}}", packets.join(","))
}

fn make_bulk_update_payload(packets: Vec<String>) -> Message {
    Message::text(make_bulk_update_packet(packets))
}

async fn handle_publish(msg: redis_async::resp::RespValue, publish_channel: String, original_keypath: String, all_clients: Clients, subscription_tracker: ManagedSubscriptionTracker) {
    let msg_str = match msg {
        redis_async::resp::RespValue::SimpleString(string) => string,
        redis_async::resp::RespValue::BulkString(bulk_string) => match str::from_utf8(&bulk_string) {
            Ok(string) => string.to_string(),
            _ => return
        },
        _ => return
    };
    
    let mut channel = publish_channel.clone();
    
    match channel.find("keypath") {
        Some(index) => channel.replace_range(0..(index + 7), ""),
        _ => ()
    };

    let channel = match channel.ends_with("/") {
        true => channel,
        false => format!("{}/", channel)
    };
    
    let clients = subscription_tracker.clients_for_keypath(original_keypath);
    let bubble_clients = subscription_tracker.clients_for_bubble_down(publish_channel);

    // println!("{:?}", bubble_clients);

    let mut packet_out: HashMap<String, Vec<String>> = HashMap::new();

    let mut pipe_packet = |client_id, packet| {
        if let Some(vec) = packet_out.get_mut(&client_id) {
            vec.push(packet);
        } else {
            let mut packets: Vec<String> = Vec::new();
            packets.push(packet);
            packet_out.insert(client_id, packets);
        }
    };

    if clients.len() > 0 {
        let message = make_update_packet(channel.to_owned(), msg_str.to_owned());

        for client_id in clients {
            pipe_packet(client_id, message.to_owned());
        }
    }

    if bubble_clients.len() > 0 {
        let mut parsed: Option<Value> = None;
        let mut bubble_cache: HashMap<String, String> = HashMap::new();

        for (client_id, bubble_path) in bubble_clients {
            let update_path = format!("{}{}/", channel, bubble_path.join("/"));

            if update_path == channel || bubble_path.len() == 0 {
                continue;
            }

            if bubble_cache.contains_key(&update_path) {
                pipe_packet(client_id, bubble_cache.get(&update_path).unwrap().to_owned());
                continue;
            }

            if parsed == None {
                parsed = match serde_json::from_str(&msg_str) {
                    Ok(value) => value,
                    _ => return
                }
            }

            let mut inner_value = parsed.to_owned().unwrap();

            for keypath in bubble_path.to_owned() {
                inner_value = match inner_value {
                    serde_json::Value::Object(ref dict) => dict[&keypath].to_owned(),
                    _ => serde_json::Value::Null
                }
            }

            let packet = make_update_packet(update_path.to_owned(), inner_value.to_string());
            bubble_cache.insert(update_path, packet.to_owned());

            pipe_packet(client_id, packet);
        }
    }

    let handles = packet_out.iter().map(|(client_id, packets)| {
        match packets.len() {
            0 => None,
            1 => Some(send_message(Message::text(packets[0].to_owned()), client_id.to_owned(), all_clients.to_owned())),
            _ => Some(send_message(make_bulk_update_payload(packets.to_vec()), client_id.to_owned(), all_clients.to_owned()))
        }
    }).into_iter().filter_map(|x| x).map(tokio::spawn).collect::<Vec<_>>();

    futures::future::join_all(handles).await;

    println!("Piped a message out to {} clients in {}ms", packet_out.len(), 0);
}

async fn diff_subscriptions(pubsub: Arc<PubsubConnection>, all_clients: Clients, subscription_tracker: ManagedSubscriptionTracker) {
    let diff_result = subscription_tracker.diff_keypaths().await;

    let new_subscribed = diff_result.new_subscribed;
    let unsubscribed = diff_result.unsubscribed;

    // println!("{:?}", new_subscribed);

    for to_subscribe in new_subscribed {
        match pubsub.psubscribe(&to_subscribe).await {
            Ok(stream) => {
                let all_clients = all_clients.to_owned();
                let subscription_tracker = subscription_tracker.to_owned();

                tokio::spawn(async move {
                    let mut event_keypath = to_subscribe.to_string();
                    event_keypath.pop();

                    stream.for_each(|msg| async {
                        if let Ok((msg, channel, _)) = msg {
                            handle_publish(msg, channel, event_keypath.to_string(), all_clients.to_owned(), subscription_tracker.to_owned()).await;
                        } else {
                            panic!("HA")
                        }
                    }).await;
                });
            },
            Err(e) => panic!("{}", e)
        }
    }
    
    for to_unsubscribe in unsubscribed {
        pubsub.punsubscribe(&to_unsubscribe);
    }
}

async fn client_connected(ws: WebSocket, stream: String, pubsub: Arc<PubsubConnection>, clients: Clients, subscription_tracker: ManagedSubscriptionTracker) {
    let (client_tx, mut client_rx) = ws.split();

    let (tx, rx) = mpsc::unbounded_channel();
    let rx = UnboundedReceiverStream::new(rx);
    tokio::task::spawn(rx.forward(client_tx));

    let id = insert_sender(tx.to_owned(), clients.to_owned());

    let send_pong = || handle_send!(tx.send(Ok(PONG_MESSAGE.to_owned())));
    let send_ok = || handle_send!(tx.send(Ok(OK_MESSAGE.to_owned())));

    while let Some(result) = client_rx.next().await {
        let payload = unwrap_option_or_break!(result.ok().and_then(|msg| msg.into_json_object()));
        let opcode = unwrap_option_or_break!(payload["o"].as_i64());

        match opcode {
            0 => {
                send_pong();
            },
            2 | 3 => {
                redis_event_track_attempt(if opcode == 2 { true } else { false }, stream.to_owned(), payload, id.to_owned(), subscription_tracker.to_owned()).await;
                send_ok();

                diff_subscriptions(pubsub.to_owned(), clients.to_owned(), subscription_tracker.to_owned()).await;
            },
            _ => break
        }
    };

    subscription_tracker.delete_client(id.to_owned());
    diff_subscriptions(pubsub.to_owned(), clients.to_owned(), subscription_tracker).await;

    clients.remove(&id);
}

macro_rules! WarpedParameter {
    ($result:ty) => {
        impl Filter<Extract = ($result,), Error = Infallible> + Clone
    };
}

fn with_pubsub(pubsub: Arc<PubsubConnection>) -> WarpedParameter!(Arc<PubsubConnection>) {
    warp::any().map(move || pubsub.to_owned())
}

fn with_clients(clients: Clients) -> WarpedParameter!(Clients) {
    warp::any().map(move || clients.to_owned())
}

fn with_subscription_tracker(subscription_tracker: ManagedSubscriptionTracker) -> WarpedParameter!(ManagedSubscriptionTracker) {
    warp::any().map(move || subscription_tracker.to_owned())
}

async fn ws_handler(stream: String, ws: warp::ws::Ws, pubsub: Arc<PubsubConnection>, clients: Clients, subscription_tracker: ManagedSubscriptionTracker) -> Result<impl warp::Reply, warp::Rejection> {
    Ok(ws.on_upgrade(move |socket| client_connected(socket, stream, pubsub, clients, subscription_tracker)))
}

use tokio::runtime::Builder;

// #[tokio::main]
fn main() {
    let runtime = Builder::new_multi_thread()
        .worker_threads(8)
        .enable_time()
        .enable_io()
        .build()
        .unwrap();

    runtime.block_on(async move {
        let clients = Clients::default();
        let subscription_tracker: ManagedSubscriptionTracker = Arc::new(SubscriptionTracker::new());

        let addr: SocketAddr = env::var("REDIS_HOST")
            .unwrap_or_else(|_| "127.0.0.1:6379".to_string())
            .parse()
            .unwrap();

        let pubsub_con = Arc::new(client::pubsub_connect(addr)
            .await
            .expect("Cannot connect to redis"));

        let routes = warp::path!("stream" / String)
            .and(warp::ws())
            .and(with_pubsub(pubsub_con))
            .and(with_clients(clients))
            .and(with_subscription_tracker(subscription_tracker))
            .and_then(ws_handler);

        warp::serve(routes)
            .run(([127, 0, 0, 1], 3030))
            .await;
    });
}