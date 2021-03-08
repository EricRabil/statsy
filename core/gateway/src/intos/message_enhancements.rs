use warp::ws::{Message};
use serde_json::{Value, Map};

pub trait IntoJson {
    fn into_json(&self) -> Option<Value>;
    fn into_json_object(&self) -> Option<Map<String, Value>>;
}

impl IntoJson for Message {
    fn into_json(&self) -> Option<Value> {
        match self.to_str() {
            Ok(msg_str) => match serde_json::from_str(msg_str) {
                Ok(value) => value,
                _ => None
            },
            _ => None
        }
    }

    fn into_json_object(&self) -> Option<Map<String, Value>> {
        match self.into_json() {
            Some(value) => match value {
                Value::Object(object) => Some(object),
                _ => None
            },
            _ => None
        }
    }
}