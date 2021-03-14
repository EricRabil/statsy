use crate::structs::Subscription;

use tokio::sync::RwLock;

pub struct SubscriptionTracker {
    pub last_subscribed_paths: Arc<RwLock<Vec<String>>>,
    pub root_subscription: Arc<Subscription>
}

use std::sync::{ Arc };

#[derive(Debug)]
pub struct SubscriptionDiff {
    pub new_subscribed: Vec<String>,
    pub unsubscribed: Vec<String>
}

fn formatted_absolute_keypath(state: String, keypath: String) -> String {
    format!("/state/{}/keypath/{}", state, keypath)
}

use std::collections::HashSet;

impl SubscriptionTracker {
    pub fn set_subscribed(&self, subscribed: bool, keypaths: Vec<String>, stream: String, id: String) {
        for key in keypaths {
            let id = id.to_owned();
            let mut key = formatted_absolute_keypath(stream.to_owned(), key);
    
            if let Some(subscription) = self.root_subscription.get_subscription(&mut key, true) {
                if subscribed {
                    subscription.add_client(id);
                } else {
                    subscription.remove_client(id);
                }
            } else {
                panic!("WAHA");
            }
        }
    }

    pub fn delete_client(&self, id: String) {
        self.root_subscription.remove_client_recursive(&id);
    }

    pub fn clients_for_keypath(&self, keypath: String) -> Vec<String> {
        let mut keypath = keypath.to_string();

        let mut observers = match self.root_subscription.get_subscription(&mut keypath, true) {
            Some(subscription) => subscription.collect_observers(),
            _ => Vec::new()
        };

        observers.sort_unstable();
        observers.dedup();
        
        observers
    }

    pub fn clients_for_bubble_down(&self, keypath: String) -> Vec<(String, Vec<String>)> {
        let mut keypath = keypath.to_string();

        println!("Getting bubbledown clients for {}", keypath);

        match self.root_subscription.get_subscription(&mut keypath, false) {
            Some(subscription) => subscription.collect_descendants(),
            _ => Vec::new()
        }
    }

    pub async fn diff_keypaths(&self) -> SubscriptionDiff {
        let mut old_keypaths = self.last_subscribed_paths.write().await;
        let new_keypaths = self.dump_keypaths();

        let new_subscribed: Vec<String> = new_keypaths.iter().filter(|keypath| !old_keypaths.contains(keypath)).map(|keypath| keypath.to_string()).collect();
        let unsubscribed: Vec<String> = old_keypaths.iter().filter(|keypath| !new_keypaths.contains(keypath)).map(|keypath| keypath.to_string()).collect();

        old_keypaths.clear();
        old_keypaths.extend(new_keypaths);

        return SubscriptionDiff {
            new_subscribed,
            unsubscribed
        }
    }

    pub fn dump_keypaths(&self) -> Vec<String> {
        let mut patterns: Vec<Vec<String>> = Vec::new();

        self.root_subscription.collect_pubsub_patterns(&mut patterns, Vec::new());

        let mut uniques: HashSet<String> = HashSet::new();

        for pattern in patterns {
            let mut joined = pattern.join("/");

            if !joined.ends_with("*") && !joined.ends_with("/") {
                joined += "/";
            }

            uniques.insert(joined);
        }

        return uniques.into_iter().collect();
    }

    pub fn new() -> SubscriptionTracker {
        SubscriptionTracker {
            last_subscribed_paths: Arc::new(RwLock::new(Vec::new())),
            root_subscription: Subscription::new()
        }
    }
}