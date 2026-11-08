//! Replacement snapshots with one pending frame per subscriber.
use knave_desktop_api::{
    DesktopError, DesktopErrorCode, DesktopEvent, DesktopResponse, DesktopSnapshot,
    MAX_SNAPSHOT_FRAME_BYTES,
};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{Arc, Mutex, Weak},
};

type Frame = Arc<[u8]>;
const MAX_PENDING_SHORTCUT_EVENTS: usize = 64;
pub(super) struct Subscriber {
    latest: Mutex<Option<Frame>>,
    event: Mutex<Option<Frame>>,
    shortcut_events: Mutex<VecDeque<Frame>>,
    wake: UnixStream,
}
impl Subscriber {
    pub(super) fn new() -> io::Result<(Arc<Self>, UnixStream)> {
        let (wake, reader) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        reader.set_nonblocking(true)?;
        Ok((
            Arc::new(Self {
                latest: Mutex::new(None),
                event: Mutex::new(None),
                shortcut_events: Mutex::new(VecDeque::new()),
                wake,
            }),
            reader,
        ))
    }
    fn wake(&self) -> bool {
        match (&self.wake).write(&[1]) {
            Ok(_) => true,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => true,
            Err(_) => false,
        }
    }
    fn publish(&self, frame: Frame) -> bool {
        let mut slot = self.latest.lock().unwrap();
        let notify = slot.is_none();
        *slot = Some(frame);
        if notify && !self.wake() {
            return false;
        }
        true
    }
    fn publish_event(&self, frame: Frame) -> bool {
        let mut slot = self.event.lock().unwrap();
        let notify = slot.is_none();
        *slot = Some(frame);
        if notify && !self.wake() {
            return false;
        }
        true
    }

    fn publish_shortcut_event(&self, frame: Frame) -> bool {
        let mut queue = self.shortcut_events.lock().unwrap();
        if queue.len() >= MAX_PENDING_SHORTCUT_EVENTS {
            queue.clear();
            let mut frame = serde_json::to_vec(&DesktopResponse::Error(DesktopError {
                code: DesktopErrorCode::Unavailable,
                message: "Global shortcut event queue overflowed".into(),
                retryable: true,
            }))
            .expect("error serializes");
            frame.push(b'\n');
            queue.push_back(frame.into());
        } else {
            queue.push_back(frame);
        }
        let notify = queue.len() == 1;
        drop(queue);
        !notify || self.wake()
    }
}

#[derive(Default)]
pub(super) struct Subscriptions {
    pub(super) snapshot: Option<DesktopSnapshot>,
    subscribers: Vec<Weak<Subscriber>>,
}
impl Subscriptions {
    pub(super) fn publish(&mut self, mut snapshot: DesktopSnapshot) {
        if self.snapshot.as_ref().is_some_and(|old| {
            old.overview_visible == snapshot.overview_visible
                && old.windows == snapshot.windows
                && old.workspaces == snapshot.workspaces
        }) {
            return;
        }
        snapshot.generation = self
            .snapshot
            .as_ref()
            .map_or(1, |old| old.generation.wrapping_add(1));
        self.snapshot = Some(snapshot);
        self.subscribers
            .retain(|subscriber| subscriber.strong_count() > 0);
        if self.subscribers.is_empty() {
            return;
        }
        let frame = self.frame();
        self.subscribers.retain(|subscriber| {
            subscriber
                .upgrade()
                .is_some_and(|s| s.publish(frame.clone()))
        });
    }
    pub(super) fn publish_event(&mut self, event: DesktopEvent) {
        self.subscribers
            .retain(|subscriber| subscriber.strong_count() > 0);
        if self.subscribers.is_empty() {
            return;
        }
        let shortcut_event = matches!(
            &event,
            DesktopEvent::GlobalShortcutActivated { .. }
                | DesktopEvent::GlobalShortcutDeactivated { .. }
        );
        let mut frame =
            serde_json::to_vec(&DesktopResponse::Event(event)).expect("event serializes");
        if frame.len() as u64 >= MAX_SNAPSHOT_FRAME_BYTES {
            frame = serde_json::to_vec(&DesktopResponse::Error(DesktopError {
                code: DesktopErrorCode::Unavailable,
                message: "Desktop event exceeds subscription capacity".into(),
                retryable: false,
            }))
            .expect("error serializes");
        }
        frame.push(b'\n');
        let frame: Frame = frame.into();
        self.subscribers.retain(|subscriber| {
            subscriber.upgrade().is_some_and(|s| {
                if shortcut_event {
                    s.publish_shortcut_event(frame.clone())
                } else {
                    s.publish_event(frame.clone())
                }
            })
        });
    }
    pub(super) fn subscribe(&mut self, subscriber: &Arc<Subscriber>) {
        self.subscribers
            .retain(|subscriber| subscriber.strong_count() > 0);
        self.subscribers.push(Arc::downgrade(subscriber));
    }
    pub(super) fn frame(&self) -> Frame {
        let snapshot = self
            .snapshot
            .as_ref()
            .expect("snapshot initialized before subscription");
        let mut frame = serde_json::to_vec(&DesktopResponse::Snapshot(snapshot.clone()))
            .expect("snapshot serializes");
        if frame.len() as u64 >= MAX_SNAPSHOT_FRAME_BYTES {
            frame = serde_json::to_vec(&DesktopResponse::Error(DesktopError {
                code: DesktopErrorCode::Unavailable,
                message: "Desktop snapshot exceeds subscription capacity".into(),
                retryable: false,
            }))
            .expect("error serializes");
        }
        frame.push(b'\n');
        frame.into()
    }
}

pub(super) fn serve(
    stream: &mut UnixStream,
    wake: &mut UnixStream,
    subscriber: &Subscriber,
) -> io::Result<()> {
    loop {
        let mut fds = [
            libc::pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // Both descriptors remain owned for the whole blocking poll; shutdown wakes it.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, -1) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        // Subscription connections accept no further requests. Readability includes EOF.
        if fds[0].revents != 0
            || fds[1].revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
        {
            return Ok(());
        }
        let mut bytes = [0; 64];
        loop {
            match wake.read(&mut bytes) {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        let snapshot_frame = subscriber.latest.lock().unwrap().take();
        let event_frame = subscriber.event.lock().unwrap().take();
        let shortcut_frames = subscriber
            .shortcut_events
            .lock()
            .unwrap()
            .drain(..)
            .collect::<Vec<_>>();
        let frames = snapshot_frame
            .into_iter()
            .chain(event_frame)
            .chain(shortcut_frames);
        for frame in frames {
            stream.write_all(&frame)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reloaded_frame(frame: &Frame) -> DesktopResponse {
        match serde_json::from_slice(frame).unwrap() {
            response @ DesktopResponse::Event(_) => response,
            other => panic!("expected an event frame: {other:?}"),
        }
    }

    fn drain_wake(wake: &mut UnixStream) -> io::Result<usize> {
        let mut bytes = [0; 64];
        match wake.read(&mut bytes) {
            Ok(n) => Ok(n),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(error) => Err(error),
        }
    }

    #[test]
    fn event_publication_wakes_and_coalesces_pending_events() {
        let mut subscriptions = Subscriptions::default();
        let (subscriber, mut wake) = Subscriber::new().unwrap();
        subscriptions.subscribe(&subscriber);

        subscriptions.publish_event(DesktopEvent::ConfigurationReloaded);
        assert_eq!(drain_wake(&mut wake).unwrap(), 1);
        // An unread event is replaced rather than queued twice.
        subscriptions.publish_event(DesktopEvent::ConfigurationReloaded);
        assert_eq!(drain_wake(&mut wake).unwrap(), 0);
        assert_eq!(
            reloaded_frame(
                &subscriber
                    .event
                    .lock()
                    .unwrap()
                    .take()
                    .expect("event frame pending")
            ),
            DesktopResponse::Event(DesktopEvent::ConfigurationReloaded)
        );
        // Publication with no subscribers does nothing and must not panic.
        drop(subscriber);
        let mut empty = Subscriptions::default();
        empty.publish_event(DesktopEvent::ConfigurationReloaded);
    }

    #[test]
    fn snapshot_and_event_frames_are_delivered_once_each() {
        let mut subscriptions = Subscriptions::default();
        let snapshot = DesktopSnapshot {
            generation: 1,
            overview_visible: false,
            workspaces: Vec::new(),
            windows: Vec::new(),
        };
        let (subscriber, mut wake) = Subscriber::new().unwrap();
        subscriptions.subscribe(&subscriber);
        subscriptions.publish(snapshot.clone());
        let _wake_held = &mut wake;

        subscriptions.publish_event(DesktopEvent::ConfigurationReloaded);
        let frames = subscriber
            .latest
            .lock()
            .unwrap()
            .take()
            .into_iter()
            .chain(subscriber.event.lock().unwrap().take())
            .collect::<Vec<_>>();
        assert_eq!(frames.len(), 2);
        assert!(matches!(
            serde_json::from_slice::<DesktopResponse>(&frames[0]).unwrap(),
            DesktopResponse::Snapshot(_)
        ));
        assert_eq!(
            reloaded_frame(&frames[1]),
            DesktopResponse::Event(DesktopEvent::ConfigurationReloaded)
        );
    }

    #[test]
    fn visibility_only_changes_publish_and_wake_subscribers() {
        let mut subscriptions = Subscriptions::default();
        let mut snapshot = DesktopSnapshot {
            generation: 0,
            overview_visible: false,
            workspaces: Vec::new(),
            windows: Vec::new(),
        };
        subscriptions.publish(snapshot.clone());
        let (subscriber, mut wake) = Subscriber::new().unwrap();
        subscriptions.subscribe(&subscriber);

        for (visible, generation) in [(true, 2), (false, 3)] {
            snapshot.overview_visible = visible;
            subscriptions.publish(snapshot.clone());
            let mut byte = [0];
            assert_eq!(wake.read(&mut byte).unwrap(), 1);
            let frame = subscriber.latest.lock().unwrap().take().unwrap();
            let DesktopResponse::Snapshot(published) = serde_json::from_slice(&frame).unwrap()
            else {
                panic!("expected a snapshot");
            };
            assert_eq!(published.overview_visible, visible);
            assert_eq!(published.generation, generation);

            subscriptions.publish(snapshot.clone());
            assert!(subscriber.latest.lock().unwrap().is_none());
            assert_eq!(
                wake.read(&mut byte).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            assert_eq!(
                subscriptions.snapshot.as_ref().unwrap().generation,
                generation
            );
        }
    }
}
