//! Private picker request and reply types shared by the portal and Shell.
use serde::{Deserialize, Serialize};
pub const MAX_REQUEST: usize = 2 * 1024 * 1024;
pub const MAX_SOURCES: usize = 64;
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
}
