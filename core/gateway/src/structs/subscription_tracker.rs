use crate::structs::MultithreadedSubscription;

pub struct SubscriptionTracker {
    pub last_subscribed_paths: Vec<String>,
    pub root_subscription: MultithreadedSubscription
}

#[derive(Debug)]
pub struct SubscriptionDiff {
    pub new_subscribed: Vec<String>,
    pub unsubscribed: Vec<String>
}

fn formatted_absolute_keypath(state: String, keypath: String) -> String {
    format!("/state/{}/keypath/{}", state, keypath)
}

impl SubscriptionTracker {
    pub async fn set_subscribed(&self, subscribed: bool, keypaths: Vec<String>, stream: String, id: String) {
        let root = self.root_subscription.write().await;

        for key in keypaths {
            let id = id.to_owned();
            let mut key = formatted_absolute_keypath(stream.to_owned(), key);
    
            if let Some(subscription) = root.get_subscription(&mut key).await {
                if subscribed {
                    subscription.write().await.add_client(id);
                } else {
                    subscription.write().await.remove_client(id);
                }
            } else {
                panic!("WAHA");
            }
        }
    }

    pub async fn delete_client(&self, id: String) {
        self.root_subscription.write().await.remove_client_recursive(id).await;
    }

    pub async fn clients_for_keypath(&self, keypath: String) -> Vec<String> {
        let root = self.root_subscription.read().await;

        let mut keypath = keypath.to_string();

        let mut observers = match root.get_subscription(&mut keypath).await {
            Some(subscription) => subscription.read().await.collect_observers().await,
            _ => Vec::new()
        };

        observers.sort_unstable();
        observers.dedup();
        
        observers
    }

    pub async fn clients_for_bubble_down(&self, keypath: String) -> Vec<(String, Vec<String>)> {
        let root = self.root_subscription.read().await;

        let mut keypath = keypath.to_string();

        println!("Getting bubbledown clients for {}", keypath);

        match root.get_subscription(&mut keypath).await {
            Some(subscription) => subscription.read().await.collect_descendants().await,
            _ => Vec::new()
        }
    }

    pub async fn diff_keypaths(&mut self) -> SubscriptionDiff {
        let old_keypaths = &self.last_subscribed_paths;
        let new_keypaths = self.dump_keypaths().await;

        let new_subscribed: Vec<String> = new_keypaths.iter().filter(|keypath| !old_keypaths.contains(keypath)).map(|keypath| keypath.to_string()).collect();
        let unsubscribed: Vec<String> = old_keypaths.iter().filter(|keypath| !new_keypaths.contains(keypath)).map(|keypath| keypath.to_string()).collect();

        self.last_subscribed_paths = new_keypaths;

        return SubscriptionDiff {
            new_subscribed,
            unsubscribed
        }
    }

    pub async fn dump_keypaths(&self) -> Vec<String> {
        let mut patterns: Vec<Vec<String>> = Vec::new();

        self.root_subscription.read().await.collect_pubsub_patterns(&mut patterns, Vec::new()).await;

        let mut assembled_patterns: Vec<String> = patterns.iter()
            .map(|pattern| {
                let mut joined = pattern.join("/");

                if !joined.ends_with("*") && !joined.ends_with("/") {
                    joined += "/";
                }

                joined
            })
            .collect();
        assembled_patterns.sort_unstable();
        assembled_patterns.dedup();

        return assembled_patterns;
    }
}