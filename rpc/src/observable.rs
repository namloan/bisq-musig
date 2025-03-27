use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::hash::Hash;
use std::prelude::rust_2021::*;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::wrappers::UnboundedReceiverStream;

#[derive(Default)]
pub struct Observable<T> {
    value: T,
    senders: Vec<mpsc::UnboundedSender<T>>,
}

struct StillObservedError<T>(Observable<T>);

impl<T> Observable<T> {
    pub const fn new(value: T) -> Self {
        Self { value, senders: Vec::new() }
    }

    fn try_into_unobserved(mut self) -> Result<T, StillObservedError<T>> {
        self.senders.retain(|s| !s.is_closed());
        if self.senders.is_empty() { Ok(self.value) } else { Err(StillObservedError(self)) }
    }
}

impl<T: Clone> Observable<T> {
    pub fn observe(&mut self) -> impl Stream<Item=T> {
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(self.value.clone()).unwrap();
        self.senders.push(tx);
        UnboundedReceiverStream::new(rx)
    }
}

impl<T: Clone + PartialEq> Observable<T> {
    pub fn replace(&mut self, value: T) -> T {
        let old_value = std::mem::replace(&mut self.value, value);
        let new_value = &self.value;
        if old_value == *new_value {
            self.senders.retain(|s| !s.is_closed());
        } else {
            self.senders.retain(|s| s.send(new_value.clone()).is_ok());
        }
        old_value
    }
}

impl<T: Clone + Default + PartialEq> Observable<T> {
    pub fn take(&mut self) -> T { self.replace(T::default()) }
}

pub struct ObservableHashMap<K, V> {
    map: HashMap<K, Observable<Option<V>>>,
}

impl<K, V> Default for ObservableHashMap<K, V> {
    fn default() -> Self { Self { map: HashMap::default() } }
}

impl<K, V> ObservableHashMap<K, V> {
    pub fn new() -> Self { Self::default() }
}

impl<K, V> ObservableHashMap<K, V>
    where K: Eq + Hash,
          V: Clone
{
    pub fn observe(&mut self, key: K) -> impl Stream<Item=Option<V>> {
        match self.map.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(Observable::default())
        }.observe()
    }
}

impl<K, V> ObservableHashMap<K, V>
    where K: Eq + Hash,
          V: Clone + PartialEq
{
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self.map.entry(key) {
            Entry::Occupied(entry) =>
                entry.into_mut().replace(Some(value)),
            Entry::Vacant(entry) => {
                entry.insert(Observable::new(Some(value)));
                None
            }
        }
    }

    pub fn remove<Q>(&mut self, k: &Q) -> Option<V>
        where K: Borrow<Q>,
              Q: Eq + Hash + ?Sized
    {
        let (key, observed_value) = self.map.remove_entry(k)?;
        observed_value.try_into_unobserved()
            .unwrap_or_else(|StillObservedError(mut o)| {
                let taken = o.take();
                self.map.insert(key, o);
                taken
            })
    }
}

impl<K, V> ObservableHashMap<K, V>
    where K: Clone + Eq + Hash,
          V: Clone + PartialEq
{
    pub fn sync(&mut self, entries: impl IntoIterator<Item=(K, V)>) {
        let mut remaining_keys: HashSet<K> = self.map.keys().cloned().collect();
        for (key, value) in entries {
            remaining_keys.remove(&key);
            self.insert(key, value);
        }
        for key in remaining_keys {
            self.remove(&key);
        }
    }
}
