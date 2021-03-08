#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use] extern crate rocket;
#[macro_use] extern crate json;

use rocket::{State,Response};
use rocket::http::{Status,ContentType};
use redis::{Connection, Commands, RedisError, RedisResult};
use std::path::{PathBuf, Component};
use std::io::Cursor;

fn ensure_state(con: &mut Connection, zone: &str) -> Result<(), RedisError> {
    if !con.exists(zone)? {
        redis::cmd("JSON.SET").arg(zone).arg(".").arg("{}").execute(con);
    }

    return Result::Ok(());
}

fn json_str_or_null(app_state: State<ApplicationState>, state: String, path: String) -> Result<String, Status> {
    match app_state.client.get_connection() {
        Ok(mut connection) => {
            let res: RedisResult<String> = redis::cmd("JSON.GET").arg(state).arg(path).query(&mut connection);

            match res {
                Ok(res) => {
                    Ok(res)
                }
                Err(_) => {
                    Ok(String::from("null"))
                }
            }
        }
        Err(_) => {
            return Err(Status::new(500, "couldn't grab connection"));
        }
    }
}

fn make_error_response<'a>(code: u16, reason: &'static str, body: json::JsonValue) -> Response<'a> {
    Response::build()
        .status(Status::new(code, reason))
        .header(ContentType::JSON)
        .streamed_body(Cursor::new(body.dump()))
        .finalize()
}

fn overwrite_json<'a>(app_state: State<ApplicationState>, state: String, original_path: &String, path: &String, json: &String) -> Result<String, Response<'a>> {
    match app_state.client.get_connection() {
        Ok(mut connection) => {
            match ensure_state(&mut connection, &state) {
                Err(_) => panic!("AHHHH"),
                _ => ()
            }

            let res: RedisResult<String> = redis::cmd("JSON.SET").arg(state.to_owned()).arg(path).arg(json).query(&mut connection);

            match res {
                Ok(_) => {
                    redis::cmd("PUBLISH").arg(format!("state/{}/keypath/{}/", state, original_path)).arg(json).execute(&mut connection);
                    Ok(json.to_string())
                }
                Err(e) => {
                    let res = e.detail().unwrap_or("null").to_string();

                    if res.starts_with("missing key") || res.starts_with("invalid key") {
                        Err(
                            make_error_response(404, "NOT FOUND", object!{
                                error: res
                            })
                        )
                    } else {
                        return Ok(res.to_string())
                    }
                }
            }
        }
        Err(_) => {
            return Err(
                make_error_response(500, "INTERNAL SERVER ERROR", object!{
                    error: "Couldn't grab connection"
                })
            );
        }
    }
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
fn index(app_state: State<ApplicationState>, state: String, subpath: PathBuf) -> Result<String, Status> {
    json_str_or_null(app_state, state, subpath_to_key_path(subpath))
}

#[get("/api/state/<state>")]
fn state(app_state: State<ApplicationState>, state: String) -> Result<String, Status> {
    index(app_state, state, PathBuf::from("/"))
}

#[post("/api/state/<state>/keypath/<subpath..>", data = "<input>")]
fn overwrite_substate(app_state: State<ApplicationState>, state: String, subpath: PathBuf, input: String) -> Result<String, Response> {
    overwrite_json(app_state, state, &subpath.as_os_str().to_str().unwrap().to_string(), &subpath_to_key_path(subpath), &input)
}

#[post("/api/state/<state>", data = "<input>")]
fn overwrite_state(app_state: State<ApplicationState>, state: String, input: String) -> Result<String, Response> {
    overwrite_substate(app_state, state, PathBuf::from("/"), input)
}

struct ApplicationState {
    client: redis::Client
}

fn main() {
    let client = redis::Client::open("redis://127.0.0.1/").unwrap();
    
    match client.get_connection() {
        Ok(mut connection) => {
            match ensure_state(&mut connection, "ericson") {
                Ok(()) => {

                }
                Err(_) => {

                }
            }
        }
        Err(_) => {

        }
    }

    rocket::ignite()
        .manage(ApplicationState { client })
        .mount("/", routes![index, state, overwrite_state, overwrite_substate])
        .launch();
}