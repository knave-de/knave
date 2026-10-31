//! Private picker request and reply types shared by the portal and Shell.
use serde::{Deserialize, Serialize};
pub const MAX_REQUEST: usize = 2 * 1024 * 1024;
pub const MAX_SOURCES: usize = 64;
pub const MAX_SHORTCUTS: usize = 64;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceKind {
    Monitor,
    Window,
    Workspace { workspace: u32 },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub preview_png: Option<String>,
    pub kind: SourceKind,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Share,
    Screenshot,
    Sharing,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_id: String,
    pub app_id: String,
    pub operation: Operation,
    pub multiple: bool,
    pub sources: Vec<Source>,
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.request_id.is_empty()
            || self.request_id.len() > 256
            || self.app_id.len() > 1024
            || self.sources.is_empty()
            || self.sources.len() > MAX_SOURCES
        {
            return Err("unsupported or oversized picker request".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut workspaces = std::collections::HashSet::new();
        for s in &self.sources {
            if !ids.insert(s.id)
                || matches!(s.kind, SourceKind::Workspace { workspace } if !(1..=32).contains(&workspace))
                || matches!(s.kind, SourceKind::Workspace { workspace } if !workspaces.insert(workspace))
                || s.name.len() > 256
                || s.description.len() > 1024
                || s.width == 0
                || s.height == 0
                || s.width > 16384
                || s.height > 16384
                || s.preview_png.as_ref().is_some_and(|p| p.len() > 192 * 1024)
            {
                return Err("invalid picker source".into());
            }
        }
        Ok(())
    }
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub request_id: String,
    pub selected: Vec<u32>,
}
impl Reply {
    pub fn validate(&self, request: &Request) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        let workspaces_only = !self.selected.is_empty()
            && self.selected.iter().all(|id| {
                request
                    .sources
                    .iter()
                    .any(|s| s.id == *id && matches!(s.kind, SourceKind::Workspace { .. }))
            });
        let has_workspace = self.selected.iter().any(|id| {
            request
                .sources
                .iter()
                .any(|s| s.id == *id && matches!(s.kind, SourceKind::Workspace { .. }))
        });
        if self.request_id != request.request_id
            || (!request.multiple && self.selected.len() > 1 && !workspaces_only)
            || (has_workspace && !workspaces_only)
            || self
                .selected
                .iter()
                .any(|id| !ids.insert(id) || !request.sources.iter().any(|s| s.id == *id))
        {
            return Err("invalid picker reply".into());
        }
        Ok(())
    }
}
/// Request shown by the shell when an application asks to configure shortcuts.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutAssignmentRequest {
    pub request_id: String,
    pub app_id: String,
    pub shortcuts: Vec<ShortcutAssignment>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutAssignment {
    pub id: String,
    pub description: String,
    pub trigger: Option<String>,
}

/// One chosen trigger, or an empty choice when the user cancels.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutAssignmentReply {
    pub request_id: String,
    pub shortcut_id: Option<String>,
    pub trigger: Option<String>,
}

impl ShortcutAssignmentRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.request_id.is_empty()
            || self.request_id.len() > 256
            || self.app_id.is_empty()
            || self.app_id.len() > 255
            || self.shortcuts.is_empty()
            || self.shortcuts.len() > MAX_SHORTCUTS
            || self.error.as_ref().is_some_and(|error| error.len() > 512)
        {
            return Err("unsupported or oversized shortcut assignment request".into());
        }
        let mut ids = std::collections::HashSet::new();
        for shortcut in &self.shortcuts {
            if shortcut.id.is_empty()
                || shortcut.id.len() > 128
                || shortcut.description.len() > 512
                || shortcut
                    .trigger
                    .as_ref()
                    .is_some_and(|trigger| trigger.len() > 64)
                || !ids.insert(&shortcut.id)
            {
                return Err("invalid shortcut assignment entry".into());
            }
        }
        Ok(())
    }
}

impl ShortcutAssignmentReply {
    pub fn validate(&self, request: &ShortcutAssignmentRequest) -> Result<(), String> {
        if self.request_id != request.request_id {
            return Err("shortcut assignment reply has the wrong request ID".into());
        }
        match (&self.shortcut_id, &self.trigger) {
            (None, None) => Ok(()),
            (Some(id), Some(trigger))
                if request.shortcuts.iter().any(|shortcut| &shortcut.id == id)
                    && !trigger.is_empty()
                    && trigger.len() <= 64 =>
            {
                Ok(())
            }
            _ => Err("invalid shortcut assignment reply".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_sets_are_one_target_and_cannot_include_other_sources() {
        let make = |id, kind| Source {
            id,
            kind,
            name: format!("source {id}"),
            description: String::new(),
            width: 1280,
            height: 800,
            preview_png: None,
        };
        let request = Request {
            request_id: "set".into(),
            app_id: "app".into(),
            operation: Operation::Share,
            multiple: false,
            sources: vec![
                make(1, SourceKind::Monitor),
                make(2, SourceKind::Workspace { workspace: 1 }),
                make(3, SourceKind::Workspace { workspace: 9 }),
            ],
        };
        assert!(request.validate().is_ok());
        let reply = |selected| Reply {
            request_id: "set".into(),
            selected,
        };
        assert!(reply(vec![2, 3]).validate(&request).is_ok());
        assert!(reply(vec![1, 2]).validate(&request).is_err());
    }

    #[test]
    fn request_and_reply_use_the_current_payload_shape() {
        let request: Request = serde_json::from_str(r#"{"request_id":"current","app_id":"app","operation":"share","multiple":false,"sources":[{"id":1,"name":"screen","description":"","width":800,"height":600,"preview_png":null,"kind":{"type":"monitor"}}]}"#).unwrap();
        assert!(request.validate().is_ok());
        assert!(
            Reply {
                request_id: "current".into(),
                selected: vec![1]
            }
            .validate(&request)
            .is_ok()
        );
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(!encoded.contains("version"));
        assert!(encoded.contains(r#""kind":{"type":"monitor"}"#));
    }
    #[test]
    fn replies_cannot_select_unknown_or_duplicate_sources() {
        let r = Request {
            request_id: "test".into(),
            app_id: "app".into(),
            operation: Operation::Share,
            multiple: true,
            sources: vec![Source {
                id: 1,
                name: "screen".into(),
                description: "".into(),
                width: 1920,
                height: 1080,
                preview_png: None,
                kind: SourceKind::Monitor,
            }],
        };
        for selected in [vec![2], vec![1, 1]] {
            assert!(
                Reply {
                    request_id: r.request_id.clone(),
                    selected
                }
                .validate(&r)
                .is_err()
            );
        }
        assert!(
            Reply {
                request_id: r.request_id.clone(),
                selected: vec![]
            }
            .validate(&r)
            .is_ok()
        );
    }
    #[test]
    fn shortcut_assignment_replies_are_bound_to_the_request() {
        let request = ShortcutAssignmentRequest {
            request_id: "request".into(),
            app_id: "example.app".into(),
            shortcuts: vec![ShortcutAssignment {
                id: "toggle".into(),
                description: "Toggle recording".into(),
                trigger: None,
            }],
            error: None,
        };
        assert!(request.validate().is_ok());
        assert!(
            ShortcutAssignmentReply {
                request_id: "request".into(),
                shortcut_id: Some("toggle".into()),
                trigger: Some("CTRL+F9".into()),
            }
            .validate(&request)
            .is_ok()
        );
        assert!(
            ShortcutAssignmentReply {
                request_id: "request".into(),
                shortcut_id: None,
                trigger: None,
            }
            .validate(&request)
            .is_ok()
        );
        assert!(
            ShortcutAssignmentReply {
                request_id: "other".into(),
                shortcut_id: Some("toggle".into()),
                trigger: Some("CTRL+F9".into()),
            }
            .validate(&request)
            .is_err()
        );
    }
}
