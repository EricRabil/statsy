#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate rocket;
#[macro_use] extern crate json;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate async_trait;

use rocket::{State,Response};
use rocket::http::{Status,ContentType, Header, Method};
use redis::AsyncCommands;
use redis::{RedisError, RedisResult};
use redis::aio::MultiplexedConnection;
use std::path::{PathBuf, Component};
use std::io::Cursor;
use std::env;
use rocket::request::{FromRequest, Outcome};
use rocket::{response, response::Responder};
use rocket::http::{HeaderMap};
use rocket::outcome::{Outcome::*};

use rocket::{Request, fairing::{Fairing, Info, Kind}};

pub struct GhettoJson(json::JsonValue);

pub struct JsonWrapper {
    inner: json::JsonValue,
    status: Status
}

struct GhettoString(String);

impl Into<GhettoString> for String {
    fn into(self) -> GhettoString {
        GhettoString(self)
    }
}

impl<'r> Into<response::Result<'r>> for GhettoString {
    fn into(self) -> response::Result<'r> {
        Response::build()
            .header(ContentType::new("application", "json"))
            .sized_body(self.0.len(), Cursor::new(self.0))
            .status(Status::default())
            .ok()
    }
}

impl<'r> Into<response::Response<'r>> for JsonWrapper {
    fn into(self) -> Response<'r> {
        let serialized = self.inner.to_string();
        Response::build()
            .header(ContentType::new("application", "json"))
            .sized_body(serialized.len(), Cursor::new(serialized))
            .status(self.status)
            .ok::<response::Result<'r>>().unwrap()
    }
}

impl<'r> Into<response::Response<'r>> for GhettoJson {
    fn into(self) -> Response<'r> {
        let serialized = self.0.to_string();
        Response::build()
            .header(ContentType::new("application", "json"))
            .sized_body(serialized.len(), Cursor::new(serialized))
            .ok::<response::Result<'r>>().unwrap()
    }
}

impl<'r> Into<response::Result<'r>> for JsonWrapper {
    fn into(self) -> response::Result<'r> {
        let serialized = self.inner.to_string();
        Response::build()
            .header(ContentType::new("application", "json"))
            .sized_body(serialized.len(), Cursor::new(serialized))
            .status(self.status)
            .ok()
    }
}

impl<'r, 'o: 'r> Responder<'r, 'o> for &'o GhettoString {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'o> {
        Response::build()
            .header(ContentType::new("application", "json"))
            .sized_body(self.0.len(), Cursor::new(self.0.to_string()))
            .status(Status::default())
            .ok()
    }
}

impl<'r, 'o: 'r> Responder<'r, 'o> for &'o JsonWrapper {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'o> {
        let serialized = self.inner.to_string();
        Response::build()
            .header(ContentType::new("application", "json"))
            .sized_body(serialized.len(), Cursor::new(serialized))
            .status(self.status)
            .ok()
    }
}

impl Into<JsonWrapper> for json::JsonValue {
    fn into(self) -> JsonWrapper {
        JsonWrapper {
            inner: self,
            status: Status::default()
        }
    }
}

pub struct CORS();

struct RocketHeaderMap<'r>(HeaderMap<'r>);

#[async_trait]
impl<'a, 'r> FromRequest<'a, 'r> for RocketHeaderMap<'r> {
    type Error = Response<'a>;

    async fn from_request(request: &'a Request<'r>) -> Outcome<Self, Self::Error> {
        Success(RocketHeaderMap(request.headers().clone()))
    }
}

#[async_trait]
impl Fairing for CORS {
    fn info(&self) -> Info {
        Info {
            name: "Add CORS headers to requests",
            kind: Kind::Response
        }
    }

    async fn on_response<'r>(&self, request: &'r Request<'_>, response: &mut Response) {
        if request.method() == Method::Options || response.content_type() == Some(ContentType::JSON) {
            response.set_header(Header::new("Access-Control-Allow-Origin", "*"));
            response.set_header(Header::new("Access-Control-Allow-Methods", "POST, GET, OPTIONS"));
            response.set_header(Header::new("Access-Control-Allow-Headers", "Content-Type, Authorization"));
            response.set_header(Header::new("Access-Control-Allow-Credentials", "true"));
        }

        if request.method() == Method::Options {
            response.set_header(ContentType::Plain);
            response.set_sized_body(0, Cursor::new(""));
        }
    }
}

mod security {
    use serde::{Serialize, Deserialize};
    use jsonwebtoken::{encode, decode, Header, Algorithm, Validation, EncodingKey, DecodingKey};
    use rocket::request::{FromRequest, Request, Outcome};
    use rocket::response::{Response};
    use rocket::outcome::{Outcome::*};
    use rocket::http::Status;
    use std::env;

    use crate::GhettoJson;

    lazy_static! {
        static ref SIGNING_KEY: String = env::var("TOKEN_SECRET").unwrap_or_else(|_| panic!("Please provide a token secret at TOKEN_SECRET env"));
        static ref ENCODING_KEY: EncodingKey = jsonwebtoken::EncodingKey::from_secret(SIGNING_KEY.as_ref());
        static ref DECODING_KEY: DecodingKey<'static> = jsonwebtoken::DecodingKey::from_secret(SIGNING_KEY.as_ref());
        static ref VALIDATION: Validation = Validation {
            leeway: 0,
            validate_exp: false,
            validate_nbf: false,
            iss: None,
            sub: None,
            aud: None,
            algorithms: vec![Algorithm::HS256]
        };
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Claims {
        pub root: bool,
        pub states: Vec<String>
    }

    #[rocket::async_trait]
    impl<'a, 'r> FromRequest<'a, 'r> for Claims {
        type Error = Response<'a>;

        async fn from_request(req: &'a Request<'r>) -> Outcome<Self, Self::Error> {
            match req.headers().get_one("authorization") {
                Some(token) => {
                    match read_token(token.to_owned()) {
                        Some(claim) => Success(claim),
                        None => Failure((Status::raw(401), GhettoJson(object!{
                            error: "That token is invalid."
                        }).into()))
                    }
                },
                None => Failure((Status::raw(401), GhettoJson(object!{
                    error: "Your token is missing."
                }).into()))
            }
        }
    }

    pub fn read_token(token: String) -> Option<Claims> {
        decode::<Claims>(&token, &DECODING_KEY, &VALIDATION).ok().map(|data| data.claims)
    }

    pub fn make_token(root: bool, states: Vec<String>) -> String {
        encode(&Header::default(), &Claims {
            root: root,
            states: states
        }, &ENCODING_KEY).unwrap()
    }
}

async fn ensure_state(con: &mut MultiplexedConnection, zone: &str) -> Result<(), RedisError> {
    match redis::cmd("JSON.SET").arg(zone).arg(".").arg("{}").arg("NX").query_async::<MultiplexedConnection, ()>(con).await {
        Err(e) => return Err(e),
        _ => ()
    }

    return Result::Ok(());
}

async fn json_str_or_null(connection: MultiplexedConnection, state: String, path: String) -> Result<String, Status> {
    let mut connection = connection.clone();
    let res: RedisResult<String> = redis::cmd("JSON.GET").arg(state).arg(path).query_async(&mut connection).await;

    match res {
        Ok(res) => {
            Ok(res)
        }
        Err(_) => {
            Ok(String::from("null"))
        }
    }
}

async fn set_json<'a>(connection: &mut MultiplexedConnection, state: String, path: String, json: String, publish_path: String) -> response::Result<'a> {
    set_json_nx(connection, state, path, json, publish_path, false).await
}

async fn set_json_nx<'a>(connection: &mut MultiplexedConnection, state: String, path: String, json: String, publish_path: String, nx: bool) -> response::Result<'a> {
    let mut cmd = redis::cmd("JSON.SET");
    cmd.arg(state.to_owned()).arg(path).arg(json.to_owned());

    if nx {
        cmd.arg("NX");
    }

    let res: RedisResult<String> = cmd.query_async(connection).await;

    if nx {
        return GhettoString(json).into()
    }

    match res {
        Ok(_) => {
            match redis::cmd("PUBLISH").arg(format!("state/{}/keypath/{}/", state, publish_path)).arg(json.to_owned()).query_async::<MultiplexedConnection, ()>(connection).await {
                Err(e) => JsonWrapper {
                    inner: object!{
                        error: e.detail().unwrap_or("Failed to publish to redis").to_string()
                    },
                    status: Status::raw(500)
                }.into(),
                Ok(_) => GhettoString(json).into()
            }
        }
        Err(e) => {
            let res = e.detail().unwrap_or("null").to_string();
            let missing = res.starts_with("missing key");
            let invalid = res.starts_with("invalid key");

            if missing || invalid {
                JsonWrapper {
                    inner: object!{
                        error: res
                    },
                    status: Status::raw(if missing { 404 } else { 400 })
                }.into()
            } else {
                return GhettoString(res).into()
            }
        }
    }
}

async fn overwrite_json<'a>(connection: MultiplexedConnection, state: String, original_path: &String, path: &String, json: &String, should_ensure_state: Option<usize>, ensure_path: Option<usize>, claims: security::Claims) -> response::Result<'a> {
    let mut connection = connection.clone();

    if !claims.root || claims.states.contains(&state) {
        return JsonWrapper {
            inner: object!{
                error: "You are not entitled to this state."
            },
            status: Status::raw(403)
        }.into()
    }

    if should_ensure_state.unwrap_or(1) == 1 {
        match ensure_state(&mut connection, &state).await {
            Err(_) => panic!("AHHHH"),
            _ => ()
        }
    }

    if ensure_path.unwrap_or(0) == 1 {
        let components_vec: Vec<&str> = path.split(".").collect();

        for (index, _) in components_vec.iter().enumerate().rev() {
            let path = components_vec[..components_vec.len() - index].join(".");
            let publish_path = components_vec[..components_vec.len() - index].join("/");
            
            match set_json_nx(&mut connection, state.to_owned(), path, "{}".to_string(), publish_path, true).await {
                Err(e) => return Err(e),
                _ => continue
            }
        }
    }

    set_json(&mut connection, state.to_owned(), path.to_owned(), json.to_owned(), original_path.to_owned()).await
}

fn subpath_to_key_path(subpath: PathBuf) -> String {
    let mut keys = vec![];

    for val in subpath.components() {
        if val == Component::RootDir {
            continue;
        }

        keys.push(val.as_os_str().to_str().unwrap());
    }

    if keys.len() == 0 {
        keys.push(".");
    }

    return keys.join(".")
}

#[get("/api/state/<state>/keypath/<subpath..>")]
async fn index<'a>(app_state: State<'a, ApplicationState>, state: String, subpath: PathBuf) -> Result<String, Status> {
    json_str_or_null(app_state.client.clone(), state, subpath_to_key_path(subpath)).await
}

#[get("/api/state/<state>")]
async fn state<'a>(app_state: State<'a, ApplicationState>, state: String) -> Result<String, Status> {
    index(app_state, state, PathBuf::from("/")).await
}

fn _inner_cors<'a>(origin: String) -> Response<'a> {
    let mut response = Response::new();

    response.set_header(Header::new("Access-Control-Allow-Origin", origin));
    response.set_header(Header::new("Access-Control-Allow-Methods", "POST, GET, OPTIONS"));
    response.set_header(Header::new("Access-Control-Allow-Headers", "Content-Type, Authorization"));
    response.set_header(Header::new("Access-Control-Allow-Credentials", "true"));
    response.set_header(Header::new("Access-Control-Max-Age", "86400"));
    response.set_header(Header::new("Content-Security-Policy", "frame-ancestors *"));

    response.set_header(ContentType::Plain);
    response.set_sized_body(0, Cursor::new(""));

    return response;
}

#[options("/api/state/<_>/keypath/<_..>")]
fn cors_keypath_inner<'a>(headers: RocketHeaderMap) -> Response<'a> {
    _inner_cors(headers.0.get_one("origin").unwrap_or("*").to_owned())
}

#[options("/api/state/<_>")]
fn cors_state<'a>(headers: RocketHeaderMap) -> Response<'a> {
    _inner_cors(headers.0.get_one("origin").unwrap_or("*").to_owned())
}

#[post("/api/state/<state>/keypath/<subpath..>?<ensure_state>&<ensure_path>", data = "<input>")]
async fn overwrite_substate<'a>(app_state: State<'a, ApplicationState>, state: String, subpath: PathBuf, input: String, ensure_state: Option<usize>, ensure_path: Option<usize>, claims: security::Claims) -> response::Result<'a> {
    overwrite_json(app_state.client.clone(), state, &subpath.as_os_str().to_str().unwrap().to_string(), &subpath_to_key_path(subpath), &input, ensure_state, ensure_path, claims).await
}

#[post("/api/state/<state>?<ensure_state>&<ensure_path>", data = "<input>")]
async fn overwrite_state<'a>(app_state: State<'a, ApplicationState>, state: String, input: String, ensure_state: Option<usize>, ensure_path: Option<usize>, claims: security::Claims) -> response::Result<'a> {
    overwrite_substate(app_state, state, PathBuf::from("/"), input, ensure_state, ensure_path, claims).await
}

struct ApplicationState {
    client: redis::aio::MultiplexedConnection
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.contains(&"--generate-token".to_string()) {
        println!("Token: {}", security::make_token(args.contains(&"--root".to_string()), vec![]));
        std::process::exit(0);
    }

    let redis_host = env::var("REDIS_HOST")
        .unwrap_or_else(|_| "localhost:6379".to_string());

    let client = redis::Client::open(format!("redis://{}", redis_host)).unwrap().get_multiplexed_async_connection().await.unwrap();

    rocket::ignite()
        .manage(ApplicationState { client })
        .mount("/", routes![index, state, overwrite_state, cors_keypath_inner, cors_state, overwrite_substate])
        .attach(CORS())
        .launch()
        .await;
}