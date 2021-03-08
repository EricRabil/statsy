use std::collections::HashMap;
use async_recursion::async_recursion;
use std::sync::{ Arc };
use tokio::sync::{ RwLock };

use crate::multithreaded::Multithreaded;
use crate::string::IndexOf;

pub type MultithreadedSubscription = Multithreaded<Subscription>;

pub struct Subscription {
    pub clients: Vec<String>,
    pub nodes: Multithreaded<HashMap<String, MultithreadedSubscription>>,
    pub parent: Option<MultithreadedSubscription>,
    rc: Option<MultithreadedSubscription>
}

const KEYPATH_TOKEN: &str = "keypath";

impl Subscription {
    pub fn add_client(&mut self, client: String) {
        if !self.clients.contains(&client) {
            self.clients.push(client);
        }
    }

    pub fn remove_client(&mut self, client: String) {
        if let Some(pos) = self.clients.iter().position(|x| *x == client) {
            self.clients.swap_remove(pos);
        }
    }

    #[async_recursion]
    pub async fn remove_client_recursive(&mut self, client: String) {
        if let Some(pos) = self.clients.iter().position(|x| *x == client) {
            self.clients.swap_remove(pos);
        }

        for (_, node) in self.nodes.read().await.iter() {
            node.write().await.remove_client_recursive(client.to_owned()).await;
        }
    }

    #[async_recursion]
    pub async fn get_subscription(&self, keypath: &mut String) -> Option<MultithreadedSubscription> {
        while keypath.starts_with("/") {
            keypath.remove(0);
        }

        while keypath.ends_with("/") {
            keypath.remove(keypath.len() - 1);
        }

        if keypath.len() == 0 {
            return self.rc.to_owned();
        }

        let next_path = match keypath.index_of('/') {
            Some(index) => {
                let tmp: String = keypath.drain(..index).collect();
                keypath.remove(0);
                Some(tmp)
            },
            _ => None
        };

        println!("next_path: {}, keypath: {}", next_path.to_owned().unwrap_or("none".to_string()), keypath);

        match next_path {
            Some(path) => {
                if self.nodes.read().await.contains_key(&path) {
                    if let Some(subscription) = self.nodes.write().await.get_mut(&path) {
                        println!("Subscription exists; Passing keypath to it for creation.");
                        return subscription.write().await.get_subscription(keypath).await;
                    } else {
                        println!("Subscription existed but failed to return.");
                        return None
                    }
                } else {
                    println!("Stub subscription does not exist. Creating and passing keypath to it.");

                    let subscription = Subscription::new();
                    let subscription_cloned = subscription.to_owned();
                    let mut subw = subscription.write().await;
        
                    subw.parent = self.rc.to_owned();
        
                    self.nodes.write().await.insert(path, subscription_cloned);
        
                    return subw.get_subscription(keypath).await;
                }
            },
            _ => {
                if self.nodes.read().await.contains_key(keypath) {
                    println!("Trunk subscription exists. Returning.");
                    return self.nodes.write().await.get(keypath).map(|arc| arc.to_owned())
                }

                println!("Trunk subscription does not exist. Creating.");

                let subscription = Subscription::new();
                let subscription_cloned = subscription.to_owned();

                self.nodes.write().await.insert(keypath.to_string(), subscription_cloned);

                return Some(subscription);
            }
        }
    }

    /// Returns all clients that should receive updates on this keypath
    /// 
    /// The logic is as follows:
    /// 
    /// Let's say this subscription is for /eric/games/active
    /// There's a subscription for client1 at /eric and /eric/games, and a subscription for client2 at /eric/games/active
    /// Updates at /eric/games/active will bubble up to /eric/games and /eric, so client1
    /// will receive an upgrade on /eric/games/active despite not being directly subscribed.
    #[async_recursion]
    pub async fn collect_observers(&self) -> Vec<String> {
        let mut observers = self.clients.clone();

        if let Some(parent) = match &self.parent {
            Some(parent) => Some(parent.read().await),
            _ => None
        } {
            observers.extend(parent.collect_observers().await);
        }

        return observers;
    }

    #[async_recursion]
    pub async fn collect_descendants(&self) -> Vec<(String, Vec<String>)> {
        let mut observers: Vec<(String, Vec<String>)> = self.clients.iter().map(|client| (client.to_owned(), Vec::new())).collect();

        for (node_id, node) in self.nodes.read().await.iter() {
            let descendants = node.read().await.collect_descendants().await;

            observers.extend::<Vec<(String, Vec<String>)>>(descendants.iter().map(|(client_id, path)| {
                let mut path = path.to_owned();
                path.insert(0, node_id.to_owned());
                (client_id.to_owned(), path)
            }).collect());
        }

        return observers;
    }

    /// Returns all keypaths that are being observed, relative to this subscription as the root
    #[async_recursion]
    pub async fn collect_pubsub_patterns(&self, into: &mut Vec<Vec<String>>, root: Vec<String>) -> bool {
        let nodes = self.nodes.read().await;

        let mut had_any = self.clients.len() > 0;
        let keypath_token_local = &KEYPATH_TOKEN.to_string();
        
        for (subpath, node) in nodes.iter() {
            let mut subroot = root.clone();
            subroot.push(subpath.to_string());

            let read_node = node.read().await;

            had_any = had_any || read_node.collect_pubsub_patterns(into, subroot.to_owned()).await;

            if had_any && subroot.contains(keypath_token_local) && read_node.clients.len() == 0 {
                into.push(subroot.clone());
            }

            if read_node.clients.len() > 0 {
                subroot.push("*".to_string());
                into.push(subroot);
            }
        }

        return had_any;
    }
}

impl Subscription {
    pub fn new() -> MultithreadedSubscription {
        return Arc::new(RwLock::new(Subscription::new_unwrapped()));
    }

    pub fn new_unwrapped() -> Subscription {
        Subscription {
            clients: Vec::new(),
            nodes: Arc::new(RwLock::new(HashMap::new())),
            parent: None,
            rc: None
        }
    }
}