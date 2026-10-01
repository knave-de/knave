//! Version 1 private picker protocol; backend and Shell must deploy together.
use serde::{Deserialize, Serialize};
pub const VERSION: u32 = 1;
pub const MAX_REQUEST: usize = 2 * 1024 * 1024;
pub const MAX_SOURCES: usize = 8;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub preview_png: Option<String>,
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
    pub version: u32,
    pub request_id: String,
    pub app_id: String,
    pub operation: Operation,
    pub multiple: bool,
    pub sources: Vec<Source>,
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != VERSION
            || self.request_id.is_empty()
            || self.request_id.len() > 256
            || self.app_id.len() > 1024
            || self.sources.is_empty()
            || self.sources.len() > MAX_SOURCES
        {
            return Err("unsupported or oversized picker request".into());
        }
        let mut ids = std::collections::HashSet::new();
        for s in &self.sources {
            if !ids.insert(s.id)
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
    pub version: u32,
    pub request_id: String,
    pub selected: Vec<u32>,
}
impl Reply {
    pub fn validate(&self, request: &Request) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        if self.version != VERSION
            || self.request_id != request.request_id
            || (!request.multiple && self.selected.len() > 1)
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
    fn replies_cannot_select_unknown_or_duplicate_sources() {
        let r = Request {
            version: VERSION,
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
            }],
        };
        for selected in [vec![2], vec![1, 1]] {
            assert!(
                Reply {
                    version: VERSION,
                    request_id: r.request_id.clone(),
                    selected
                }
                .validate(&r)
                .is_err()
            );
        }
        assert!(
            Reply {
                version: VERSION,
                request_id: r.request_id.clone(),
                selected: vec![]
            }
            .validate(&r)
            .is_ok()
        );
    }
}
