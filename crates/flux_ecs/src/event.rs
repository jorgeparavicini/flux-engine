//! Buffered events with per-reader cursors.
//!
//! Each event type has one append-only buffer carrying a monotonic `u64`
//! sequence. A reader remembers the sequence it last consumed, so it sees each
//! event exactly once no matter how often it runs. Events before the slowest
//! registered reader's cursor are reclaimed, bounding the buffer.

use crate::access::AccessList;
use crate::system::SystemParam;
use crate::world::{World, WorldCells};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

/// One event type's buffer: the retained events plus every reader's cursor.
pub(crate) struct EventStore<E> {
    events: VecDeque<E>,
    /// Sequence number of `events.front()`.
    start: u64,
    /// One cursor per registered reader; the next sequence it will read.
    cursors: Vec<AtomicU64>,
}

impl<E> Default for EventStore<E> {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            start: 0,
            cursors: Vec::new(),
        }
    }
}

impl<E> EventStore<E> {
    /// The sequence the next sent event will take.
    fn next_seq(&self) -> u64 {
        self.start + self.events.len() as u64
    }

    fn send(&mut self, event: E) {
        self.events.push_back(event);
    }

    /// Registers a reader starting at the current end, returning its index.
    fn register(&mut self) -> usize {
        let id = self.cursors.len();
        self.cursors.push(AtomicU64::new(self.next_seq()));
        id
    }

    /// Drops events no registered reader can still reach.
    fn reclaim(&mut self) {
        let horizon = self
            .cursors
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .min()
            .unwrap_or_else(|| self.next_seq());
        while self.start < horizon {
            self.events.pop_front();
            self.start += 1;
        }
    }

    /// The events reader `id` has not yet seen, advancing its cursor past them.
    fn read(&self, id: usize) -> impl Iterator<Item = &E> {
        let cursor = &self.cursors[id];
        let from = (cursor.load(Ordering::Relaxed) - self.start) as usize;
        cursor.store(self.next_seq(), Ordering::Relaxed);
        self.events.iter().skip(from)
    }
}

/// Per-world set of event buffers, keyed by event type.
#[derive(Default)]
pub(crate) struct EventRegistry {
    stores: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl EventRegistry {
    fn store<E: Send + Sync + 'static>(&self) -> Option<&EventStore<E>> {
        self.stores.get(&TypeId::of::<E>())?.downcast_ref()
    }

    fn store_mut<E: Send + Sync + 'static>(&mut self) -> &mut EventStore<E> {
        self.stores
            .entry(TypeId::of::<E>())
            .or_insert_with(|| Box::<EventStore<E>>::default())
            .downcast_mut()
            .expect("keyed by TypeId")
    }

    pub(crate) fn send<E: Send + Sync + 'static>(&mut self, event: E) {
        self.store_mut::<E>().send(event);
    }

    pub(crate) fn register_reader<E: Send + Sync + 'static>(&mut self) -> usize {
        self.store_mut::<E>().register()
    }

    pub(crate) fn reclaim<E: Send + Sync + 'static>(&mut self) {
        self.store_mut::<E>().reclaim();
    }

    /// Reader `id`'s unseen events, advancing its cursor. Empty if the type
    /// has no buffer yet.
    pub(crate) fn read<E: Send + Sync + 'static>(&self, id: usize) -> impl Iterator<Item = &E> {
        self.store::<E>().into_iter().flat_map(move |s| s.read(id))
    }
}

/// System parameter for sending events of type `E`.
///
/// Sends are buffered per system and flushed after the system runs, so writers
/// and readers never contend during a run.
pub struct EventWriter<'s, E>(&'s mut Vec<E>);

impl<E> EventWriter<'_, E> {
    /// Queues `event` for delivery to readers.
    pub fn send(&mut self, event: E) {
        self.0.push(event);
    }
}

unsafe impl<E: Send + Sync + 'static> SystemParam for EventWriter<'_, E> {
    const ACCESS: AccessList = AccessList::EMPTY;
    type State = Vec<E>;
    type Item<'w, 's> = EventWriter<'s, E>;

    fn init(_world: &mut World) -> Self::State {
        Vec::new()
    }

    unsafe fn fetch<'w, 's>(
        state: &'s mut Self::State,
        _cells: &WorldCells<'w>,
        _version: u64,
    ) -> Self::Item<'w, 's> {
        EventWriter(state)
    }

    fn apply(state: &mut Self::State, world: &mut World) {
        for event in state.drain(..) {
            world.send_event(event);
        }
        world.reclaim_events::<E>();
    }
}

/// System parameter for reading events of type `E`.
///
/// Each reader remembers where it left off, so it sees every event exactly
/// once across its runs regardless of how often it polls.
pub struct EventReader<'w, E> {
    events: &'w EventRegistry,
    id: usize,
    _marker: PhantomData<fn() -> E>,
}

impl<E: Send + Sync + 'static> EventReader<'_, E> {
    /// The events sent since this reader last read, marking them all as seen.
    pub fn read(&mut self) -> impl Iterator<Item = &E> {
        self.events.read::<E>(self.id)
    }
}

unsafe impl<E: Send + Sync + 'static> SystemParam for EventReader<'_, E> {
    const ACCESS: AccessList = AccessList::EMPTY;
    type State = usize;
    type Item<'w, 's> = EventReader<'w, E>;

    fn init(world: &mut World) -> Self::State {
        world.register_event_reader::<E>()
    }

    unsafe fn fetch<'w, 's>(
        state: &'s mut Self::State,
        cells: &WorldCells<'w>,
        _version: u64,
    ) -> Self::Item<'w, 's> {
        EventReader {
            events: cells.events,
            id: *state,
            _marker: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_sees_each_event_once() {
        let mut store = EventStore::<u32>::default();
        let r = store.register();
        store.send(1);
        store.send(2);
        assert_eq!(store.read(r).copied().collect::<Vec<_>>(), vec![1, 2]);
        assert!(store.read(r).next().is_none(), "already consumed");
        store.send(3);
        assert_eq!(store.read(r).copied().collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn readers_advance_independently() {
        let mut store = EventStore::<u32>::default();
        let fast = store.register();
        let slow = store.register();
        store.send(10);
        store.send(20);
        assert_eq!(store.read(fast).copied().collect::<Vec<_>>(), vec![10, 20]);
        // slow has not read yet; both still retained.
        store.reclaim();
        assert_eq!(store.read(slow).copied().collect::<Vec<_>>(), vec![10, 20]);
    }

    #[test]
    fn reclaim_drops_only_fully_seen_events() {
        let mut store = EventStore::<u32>::default();
        let fast = store.register();
        let slow = store.register();
        store.send(1);
        store.send(2);
        store.read(fast).for_each(drop);
        store.reclaim(); // slow still at 0, nothing dropped
        assert_eq!(store.start, 0);
        store.read(slow).for_each(drop);
        store.send(3);
        store.reclaim(); // both past 1 and 2
        assert_eq!(store.start, 2);
        // The late reader still sees the survivor exactly once.
        assert_eq!(store.read(fast).copied().collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn a_new_reader_ignores_past_events() {
        let mut store = EventStore::<u32>::default();
        store.send(1);
        let r = store.register();
        assert!(store.read(r).next().is_none());
        store.send(2);
        assert_eq!(store.read(r).copied().collect::<Vec<_>>(), vec![2]);
    }

    use crate::{IntoSystem, System, World};
    use std::cell::RefCell;

    thread_local! {
        static SEEN: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    }

    #[derive(Debug, PartialEq)]
    struct Ping(u32);

    fn producer(mut writer: EventWriter<Ping>) {
        writer.send(Ping(1));
        writer.send(Ping(2));
    }

    fn consumer(mut reader: EventReader<Ping>) {
        SEEN.with(|s| s.borrow_mut().extend(reader.read().map(|p| p.0)));
    }

    #[test]
    fn writer_and_reader_systems_deliver_each_event_once() {
        SEEN.with(|s| s.borrow_mut().clear());
        let mut world = World::new();

        // Register the reader before any event is sent so it starts at zero.
        let mut reader = consumer.into_system();
        reader.initialize(&mut world);
        let mut writer = producer.into_system();

        world.run(&mut writer);
        world.run(&mut reader);
        assert_eq!(SEEN.with(|s| s.borrow().clone()), vec![1, 2]);

        // A second frame delivers the next batch, once.
        world.run(&mut writer);
        world.run(&mut reader);
        assert_eq!(SEEN.with(|s| s.borrow().clone()), vec![1, 2, 1, 2]);

        // Reading again with nothing new yields nothing.
        world.run(&mut reader);
        assert_eq!(SEEN.with(|s| s.borrow().clone()), vec![1, 2, 1, 2]);
    }

    static SEEN_TOTAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    #[derive(Debug)]
    struct Tick;

    fn tick_writer(mut writer: EventWriter<Tick>) {
        for _ in 0..4 {
            writer.send(Tick);
        }
    }

    fn reader_a(mut reader: EventReader<Tick>) {
        SEEN_TOTAL.fetch_add(reader.read().count(), Ordering::Relaxed);
    }

    fn reader_b(mut reader: EventReader<Tick>) {
        SEEN_TOTAL.fetch_add(reader.read().count(), Ordering::Relaxed);
    }

    #[test]
    fn parallel_readers_each_see_every_event_once() {
        use crate::{Schedule, SystemLabel};
        const WRITE: SystemLabel = SystemLabel("write");
        SEEN_TOTAL.store(0, Ordering::Relaxed);

        let mut world = World::new();
        let mut schedule = Schedule::new();
        schedule.add(tick_writer).label(WRITE);
        schedule.add(reader_a).after(WRITE);
        schedule.add(reader_b).after(WRITE);

        for _ in 0..3 {
            schedule.run_parallel(&mut world);
        }
        // Two readers, three frames, four events each frame.
        assert_eq!(SEEN_TOTAL.load(Ordering::Relaxed), 2 * 3 * 4);
    }
}
