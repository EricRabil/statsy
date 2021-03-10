use std::sync::{ Arc };

use crate::string::IndexOf;

use dashmap::{DashMap, DashSet};

pub struct Subscription {
    pub clients: DashSet<String>,
    pub nodes: DashMap<String, Arc<Subscription>>,
    pub parent: Option<Arc<Subscription>>,
    rc: Option<Arc<Subscription>>
}

const KEYPATH_TOKEN: &str = "keypath";

impl Subscription {
    pub fn add_client(&self, client: String) {
        if !self.clients.contains(&client) {
            self.clients.insert(client);
        }
    }

    pub fn remove_client(&self, client: String) {
        self.clients.remove(&client);
    }

    pub fn remove_client_recursive(&self, client: &String) {
        self.clients.remove(client);

        for node in self.nodes.to_owned().into_read_only().values() {
            node.remove_client_recursive(&client);
        }
    }

    pub fn get_subscription(&self, keypath: &mut String, create: bool) -> Option<Arc<Subscription>> {
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

        // println!("next_path: {}, keypath: {}", next_path.to_owned().unwrap_or("none".to_string()), keypath);

        match next_path {
            Some(path) => {
                if self.nodes.contains_key(&path) {
                    if let Some(subscription) = self.nodes.get(&path) {
                        // println!("Subscription exists; Passing keypath to it for creation.");
                        return subscription.get_subscription(keypath, create);
                    } else {
                        // println!("Subscription existed but failed to return.");
                        return None
                    }
                } else if create {
                    // println!("Stub subscription does not exist. Creating and passing keypath to it.");

                    let subscription = Subscription::new();

                    unsafe {
                        let mut subscription_arc_mut = subscription.to_owned();
                        let mut subw = Arc::get_mut_unchecked(&mut subscription_arc_mut);
                        subw.parent = self.rc.to_owned();
                        
                        self.nodes.insert(path, subscription);
            
                        return subw.get_subscription(keypath, create);
                    }
                } else {
                    return None
                }
            },
            _ => {
                if self.nodes.contains_key(keypath) {
                    // println!("Trunk subscription exists. Returning.");
                    return self.nodes.get(keypath).map(|arc| arc.to_owned())
                } else if !create {
                    // println!("Trunk subscription does not exist but we were instructed not to create.");
                    return None
                }

                // println!("Trunk subscription does not exist. Creating.");

                let subscription = Subscription::new();
                let subscription_cloned = subscription.to_owned();

                self.nodes.insert(keypath.to_string(), subscription_cloned);

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
    pub fn collect_observers(&self) -> Vec<String> {
        let mut observers: Vec<String> = self.clients.iter().map(|pair| pair.key().to_string()).collect();

        if let Some(parent) = &self.parent {
            observers.extend(parent.collect_observers());
        }

        return observers;
    }

    pub fn collect_descendants(&self) -> Vec<(String, Vec<String>)> {
        let mut observers: Vec<(String, Vec<String>)> = self.clients.iter().map(|client| (client.to_owned(), Vec::new())).collect();

        for (node_id, node) in self.nodes.to_owned().into_read_only().iter() {
            let descendants = node.collect_descendants();

            observers.extend::<Vec<(String, Vec<String>)>>(descendants.iter().map(|(client_id, path)| {
                let mut path = path.to_owned();
                path.insert(0, node_id.to_owned());
                (client_id.to_owned(), path)
            }).collect());
        }

        return observers;
    }

    /// Returns all keypaths that are being observed, relative to this subscription as the root
    pub fn collect_pubsub_patterns(&self, into: &mut Vec<Vec<String>>, root: Vec<String>) -> bool {
        let mut had_any = self.clients.len() > 0;
        let keypath_token_local = &KEYPATH_TOKEN.to_string();
        
        for (node_id, node) in self.nodes.to_owned().into_read_only().iter() {
            let mut subroot = root.clone();
            subroot.push(node_id.to_owned());

            had_any = had_any || node.collect_pubsub_patterns(into, subroot.to_owned());

            if had_any && subroot.contains(keypath_token_local) && node.clients.len() == 0 {
                into.push(subroot.clone());
            }

            if node.clients.len() > 0 {
                subroot.push("*".to_string());
                into.push(subroot);
            }
        }

        return had_any;
    }
}

impl Subscription {
    pub fn new() -> Arc<Subscription> {
        let subscription = Arc::new(Subscription::new_unwrapped());

        unsafe {
            let mut subscription_mut = subscription.to_owned();
            Arc::get_mut_unchecked(&mut subscription_mut).rc = Some(subscription.to_owned());
        }

        subscription
    }

    pub fn new_unwrapped() -> Subscription {
        Subscription {
            clients: DashSet::new(),
            nodes: DashMap::new(),
            parent: None,
            rc: None
        }
    }
}