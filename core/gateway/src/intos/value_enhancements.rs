use warp::ws::{Message};
use serde_json::{Value};

pub trait IntoStrVec {
    fn into_str_vec(&self) -> Option<Vec<&str>>;
}

pub trait IntoMessage {
    fn into_message(&self) -> Message;
}

pub trait IntoStringVec {
    fn into_string_vec(&self) -> Option<Vec<String>>;
}

impl IntoStrVec for Value {
    fn into_str_vec(&self) -> Option<Vec<&str>> {
        self.as_array().map(|values| values.iter().filter_map(|value| value.as_str()).collect())
    }
}

impl IntoStringVec for Value {
    fn into_string_vec(&self) -> Option<Vec<String>> {
        self.into_str_vec().map(|vec| vec.iter().map(|str| str.to_string()).collect())
    }
}

impl IntoMessage for Value {
    fn into_message(&self) -> Message {
        Message::text(self.to_string())
    }
}