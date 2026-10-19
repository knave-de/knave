use knave_config::{Config, notifications::AppPolicy};
use knave_desktop_api::notifications::*;
use std::collections::HashMap;

#[derive(Clone)]
pub(super) struct Live {
    pub protocol_id: u32,
    pub sender: String,
    pub resident: bool,
    pub transient: bool,
    pub expires: Option<u64>,
    pub banner_until: Option<u64>,
}
#[derive(Clone)]
pub(super) struct Model {
    pub snapshot: NotificationSnapshot,
    pub live: HashMap<u64, Live>,
    pub next: u64,
    pub config: Config,
}
pub(super) struct Incoming {
    pub sender: String,
    pub replaces: u32,
    pub app: String,
    pub icon: String,
    pub image: Option<NotificationImage>,
    pub summary: String,
    pub body: String,
    pub app_id: String,
    pub actions: Vec<NotificationAction>,
    pub urgency: u8,
    pub transient: bool,
    pub resident: bool,
    pub progress: Option<u8>,
    pub timeout: i32,
}
#[derive(Debug)]
pub(super) enum Signal {
    Closed(u32, u32, String),
    Action(u32, String, String),
}
impl Model {
    pub fn new(config: Config, entries: Vec<Notification>, dnd: bool, next: u64) -> Self {
        Self {
            snapshot: NotificationSnapshot {
                owned: false,
                dnd,
                light: config.appearance.theme.color_scheme() == 2
                    || (config.appearance.theme.color_scheme() == 0
                        && config.portal.color_scheme == 2),
                high_contrast: config.portal.high_contrast,
                history_days: config.notifications.history_days,
                history_limit: config.notifications.history_limit,
                app_banners: config
                    .notifications
                    .app
                    .iter()
                    .map(|(id, p)| (id.clone(), p.banners))
                    .collect(),
                notifications: entries,
                ..Default::default()
            },
            live: HashMap::new(),
            next,
            config,
        }
    }
    pub fn policy(&self, id: &str) -> AppPolicy {
        self.config
            .notifications
            .app
            .get(id)
            .cloned()
            .unwrap_or_default()
    }
    pub fn notify(&mut self, incoming: Incoming, now: u64) -> Result<(u32, Vec<Signal>), String> {
        if !self.snapshot.owned {
            return Err("another notification service owns delivery".into());
        }
        if incoming.timeout < -1 {
            return Err("expire_timeout must be -1, 0 or positive".into());
        }
        let existing = self
            .live
            .iter()
            .find(|(_, l)| l.protocol_id == incoming.replaces);
        let id = if let Some((id, live)) = existing {
            if live.sender != incoming.sender {
                return Err("cannot replace another application's notification".into());
            }
            *id
        } else {
            if self.next > u32::MAX as u64 {
                return Err("notification IDs exhausted".into());
            }
            if self.live.len() >= MAX_LIVE {
                return Err("too many live notifications; retry after closing one".into());
            }
            let id = self.next;
            self.next += 1;
            id
        };
        let policy = self.policy(&incoming.app_id);
        let mut signals = Vec::new();
        self.snapshot.notifications.retain(|n| n.id != id);
        let critical = incoming.urgency == 2;
        let timeout = if incoming.timeout == -1 {
            if critical {
                0
            } else {
                self.config.notifications.banner_timeout_ms
            }
        } else {
            incoming.timeout as u32
        };
        let banner = policy.enabled
            && policy.banners
            && !self.snapshot.center_open
            && (!self.snapshot.dnd || (critical && self.config.notifications.critical_in_dnd));
        let n = Notification {
            id,
            app: incoming.app,
            app_id: incoming.app_id,
            icon: incoming.icon,
            image: incoming.image,
            summary: incoming.summary,
            body: incoming.body,
            created_ms: now,
            unread: true,
            urgency: incoming.urgency,
            progress: incoming.progress,
            live: true,
            transient: incoming.transient,
            banner,
            actions: incoming.actions,
        };
        self.live.insert(
            id,
            Live {
                protocol_id: id as u32,
                sender: incoming.sender,
                resident: incoming.resident,
                transient: incoming.transient,
                expires: (timeout != 0).then(|| now.saturating_add(u64::from(timeout))),
                banner_until: (!critical).then(|| {
                    now.saturating_add(u64::from(self.config.notifications.banner_timeout_ms))
                }),
            },
        );
        self.snapshot.notifications.insert(0, n);
        if !policy.enabled {
            self.remove(id, 2, &mut signals);
        } // Explicit app policy suppresses both history and presentation.
        self.prune(now, &mut signals);
        Ok((id as u32, signals))
    }
    fn finish(&mut self, id: u64, reason: u32, signals: &mut Vec<Signal>) {
        if let Some(l) = self.live.remove(&id) {
            signals.push(Signal::Closed(l.protocol_id, reason, l.sender));
            if l.transient {
                self.snapshot.notifications.retain(|n| n.id != id);
            } else if let Some(n) = self.snapshot.notifications.iter_mut().find(|n| n.id == id) {
                n.live = false;
                n.banner = false;
                n.actions.clear();
            }
        }
    }
    pub fn remove(&mut self, id: u64, reason: u32, signals: &mut Vec<Signal>) {
        self.finish(id, reason, signals);
        self.snapshot.notifications.retain(|n| n.id != id);
    }
    pub fn sender_gone(&mut self, sender: &str) -> bool {
        let mut changed = false;
        for n in &mut self.snapshot.notifications {
            if self.live.get(&n.id).is_some_and(|l| l.sender == sender) && !n.actions.is_empty() {
                n.actions.clear();
                changed = true;
            }
        }
        changed
    }
    pub fn close(&mut self, id: u32, sender: &str) -> Result<Vec<Signal>, String> {
        let key = self
            .live
            .iter()
            .find(|(_, l)| l.protocol_id == id)
            .map(|(k, l)| (*k, l.sender.clone()))
            .ok_or("notification no longer exists")?;
        if key.1 != sender {
            return Err("cannot close another application's notification".into());
        }
        let mut signals = Vec::new();
        self.remove(key.0, 3, &mut signals);
        Ok(signals)
    }
    pub fn command(&mut self, command: NotificationCommand) -> Result<Vec<Signal>, String> {
        let mut signals = Vec::new();
        match command {
            NotificationCommand::ToggleCenter => {
                self.snapshot.center_open = !self.snapshot.center_open;
                if self.snapshot.center_open {
                    for n in &mut self.snapshot.notifications {
                        n.banner = false;
                    }
                }
            }
            NotificationCommand::CloseCenter => self.snapshot.center_open = false,
            NotificationCommand::SetDnd { enabled } => {
                self.snapshot.dnd = enabled;
                if enabled {
                    for n in &mut self.snapshot.notifications {
                        n.banner = false;
                    }
                }
            }
            NotificationCommand::Seen { ids } => {
                if ids.len() > MAX_HISTORY + MAX_LIVE {
                    return Err("too many notification IDs".into());
                }
                for n in &mut self.snapshot.notifications {
                    if ids.contains(&n.id) {
                        n.unread = false;
                        n.banner = false;
                    }
                }
            }
            NotificationCommand::Dismiss { id } => self.remove(id, 2, &mut signals),
            NotificationCommand::Clear => {
                let ids: Vec<_> = self.snapshot.notifications.iter().map(|n| n.id).collect();
                for id in ids {
                    self.remove(id, 2, &mut signals);
                }
            }
            NotificationCommand::Invoke { id, action } => {
                let live = self
                    .live
                    .get(&id)
                    .ok_or("archived notification actions are unavailable")?;
                let n = self
                    .snapshot
                    .notifications
                    .iter_mut()
                    .find(|n| n.id == id)
                    .ok_or("notification no longer exists")?;
                if !n.actions.iter().any(|a| a.key == action) {
                    return Err("unknown notification action".into());
                }
                signals.push(Signal::Action(
                    live.protocol_id,
                    action,
                    live.sender.clone(),
                ));
                n.unread = false;
                if !live.resident {
                    self.remove(id, 2, &mut signals);
                }
            }
            NotificationCommand::SetAppBanners { app_id, enabled } => {
                if app_id.is_empty() || app_id.len() > 256 || app_id.chars().any(char::is_control) {
                    return Err("invalid application identity".into());
                }
                let mut doc =
                    knave_config::ConfigDocument::at_default_path().map_err(|e| e.to_string())?;
                let mut config = doc.config().clone();
                config
                    .notifications
                    .app
                    .entry(app_id.clone())
                    .or_default()
                    .banners = enabled;
                doc.write(config.clone()).map_err(|e| e.to_string())?;
                self.config.notifications.app = config.notifications.app;
                self.snapshot.app_banners.insert(app_id.clone(), enabled);
                if !enabled {
                    for n in &mut self.snapshot.notifications {
                        if n.app_id == app_id {
                            n.banner = false;
                        }
                    }
                }
            }
        }
        Ok(signals)
    }
    pub fn tick(&mut self, now: u64) -> Vec<Signal> {
        let mut signals = Vec::new();
        let expired: Vec<_> = self
            .live
            .iter()
            .filter(|(_, l)| l.expires.is_some_and(|t| t <= now))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            self.finish(id, 1, &mut signals);
        }
        for n in &mut self.snapshot.notifications {
            if self
                .live
                .get(&n.id)
                .is_some_and(|l| l.banner_until.is_some_and(|t| t <= now))
            {
                n.banner = false;
            }
        }
        self.prune(now, &mut signals);
        signals
    }
    pub fn prune(&mut self, now: u64, signals: &mut Vec<Signal>) {
        let cutoff =
            now.saturating_sub(u64::from(self.config.notifications.history_days) * 86_400_000);
        let mut kept = 0;
        let mut remove = Vec::new();
        for n in &self.snapshot.notifications {
            let transient = self.live.get(&n.id).is_some_and(|l| l.transient);
            if !transient {
                kept += 1;
                if kept > self.config.notifications.history_limit || n.created_ms < cutoff {
                    remove.push(n.id);
                }
            }
        }
        for id in remove {
            self.remove(id, 1, signals);
        }
        let mut image_bytes = 0;
        for n in &mut self.snapshot.notifications {
            if let Some(image) = &n.image {
                if image_bytes + image.pixels().len() <= MAX_TOTAL_IMAGE_BYTES {
                    image_bytes += image.pixels().len();
                } else {
                    n.image = None;
                }
            }
        }
        let mut banners = 0;
        for n in &mut self.snapshot.notifications {
            if n.banner {
                banners += 1;
                if banners > self.config.notifications.max_banners {
                    n.banner = false;
                }
            }
        }
    }
    pub fn deadline(&self) -> Option<u64> {
        let live = self
            .live
            .iter()
            .flat_map(|(id, l)| {
                [
                    l.expires,
                    l.banner_until.filter(|_| {
                        self.snapshot
                            .notifications
                            .iter()
                            .any(|n| n.id == *id && n.banner)
                    }),
                ]
            })
            .flatten();
        let age = self
            .snapshot
            .notifications
            .iter()
            .filter(|n| !self.live.get(&n.id).is_some_and(|l| l.transient))
            .map(|n| {
                n.created_ms.saturating_add(
                    u64::from(self.config.notifications.history_days) * 86_400_000 + 1,
                )
            });
        live.chain(age).min()
    }
    pub fn durable(&self) -> Vec<Notification> {
        self.snapshot
            .notifications
            .iter()
            .filter(|n| !self.live.get(&n.id).is_some_and(|l| l.transient))
            .cloned()
            .map(|mut n| {
                n.live = false;
                n.banner = false;
                n.actions.clear();
                n
            })
            .collect()
    }
}

pub(super) fn bounded(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .scan(0, |bytes, c| {
            *bytes += c.len_utf8();
            (*bytes <= max).then_some(c)
        })
        .collect()
}
