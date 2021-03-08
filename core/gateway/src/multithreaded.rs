use std::sync::{
    Arc
};
use tokio::sync::{RwLock};

pub type Multithreaded<T> = Arc<RwLock<T>>;