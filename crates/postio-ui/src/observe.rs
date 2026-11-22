//! What an app looks like to a storyboard: one `Observation` shape for every frontend (spec 008),
//! so one storyboard checks them all. A field an app cannot observe is `None`, never a guess.

// Every variant and field is documented once, in data-model.md's table; a
// second doc line on each would be two homes for one fact.
#![allow(missing_docs)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Window {
    Open,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    List,
    Conversation,
    Reader,
    Search,
    Composer,
    Settings,
    FirstRun,
    Locked,
    Digest,
    Filtered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    Sidebar,
    List,
    Reader,
    Conversation,
    Composer,
    Search,
    Results,
    Palette,
    Picker,
    Cheatsheet,
    Settings,
    Dialog,
    Menu,
    Banner,
    None,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Overlay {
    None,
    Palette,
    Finder,
    Picker,
    Cheatsheet,
    Dialog,
    Menu,
    Keymap,
    Popover,
}

/// Recorded when shown, not inferred from the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyboard {
    pub region: Region,
    pub field: Option<String>,
    pub typing: bool,
    pub reachable: bool,
    /// Informative only: it names a toolkit's internals, so `Observation::shared_eq` leaves it out
    /// and checks may not use it.
    pub widget: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub index: Option<u32>,
    pub id: Option<String>,
    pub subject: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rows {
    pub first_visible: Option<u32>,
    pub count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayState {
    pub kind: Overlay,
    pub mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub text: Option<String>,
    pub tone: Option<Tone>,
    pub undo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Banner {
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scroll {
    pub offset: u32,
    pub max: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reading {
    pub id: Option<String>,
    pub focused: Option<u32>,
    pub scroll: Option<Scroll>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Composer {
    pub open: bool,
    pub detached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub window: Window,
    pub view: View,
    pub scope: Option<String>,
    pub keyboard: Keyboard,
    pub cursor: Cursor,
    pub rows: Rows,
    pub selection: Selection,
    pub overlay: OverlayState,
    pub notice: Notice,
    pub banner: Banner,
    pub reading: Reading,
    pub composer: Composer,
    pub back_depth: Option<u32>,
    /// Namespaced app-specific fields (`classic.pane`); shared storyboards may not check them, so
    /// they do not compare.
    #[serde(default)]
    pub app: BTreeMap<String, serde_json::Value>,
}

impl Observation {
    pub fn shared_eq(&self, other: &Self) -> bool {
        let mut a = self.clone();
        let mut b = other.clone();
        for o in [&mut a, &mut b] {
            o.keyboard.widget.clear();
            o.app.clear();
        }
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Observation {
        Observation {
            window: Window::Open,
            view: View::FirstRun,
            scope: Some("Inbox".into()),
            keyboard: Keyboard {
                region: Region::Cheatsheet,
                field: Some("subject".into()),
                typing: true,
                reachable: true,
                widget: "Window/Box/Entry".into(),
            },
            cursor: Cursor {
                index: Some(2),
                id: Some("m-3".into()),
                subject: Some("Quarterly plan".into()),
            },
            rows: Rows {
                first_visible: Some(0),
                count: Some(40),
            },
            selection: Selection { count: 1 },
            overlay: OverlayState {
                kind: Overlay::Keymap,
                mode: Some("rules".into()),
            },
            notice: Notice {
                text: Some("Archived".into()),
                tone: Some(Tone::Success),
                undo: true,
            },
            banner: Banner {
                title: Some("Offline".into()),
            },
            reading: Reading {
                id: Some("t-1".into()),
                focused: Some(1),
                scroll: Some(Scroll {
                    offset: 10,
                    max: 900,
                }),
            },
            composer: Composer {
                open: true,
                detached: false,
            },
            back_depth: Some(2),
            app: BTreeMap::from([("classic.pane".to_string(), serde_json::json!("list"))]),
        }
    }

    #[test]
    fn a_fully_populated_observation_survives_json() {
        let o = full();
        let text = serde_json::to_string(&o).unwrap();
        assert_eq!(serde_json::from_str::<Observation>(&text).unwrap(), o);
    }

    #[test]
    fn shared_eq_ignores_the_widget_path_and_app_fields() {
        let a = full();
        let mut b = full();
        b.keyboard.widget = "Other/Path".into();
        b.app.insert("focus.bulk".into(), serde_json::json!(true));
        b.app.remove("classic.pane");
        assert!(a.shared_eq(&b), "widget and app.* must not count");
        assert_ne!(a, b, "plain equality still sees them");
    }

    #[test]
    fn shared_eq_compares_every_other_field() {
        let a = full();
        type Change = Box<dyn Fn(&mut Observation)>;
        let changes: Vec<(&str, Change)> = vec![
            ("window", Box::new(|o| o.window = Window::Closed)),
            ("view", Box::new(|o| o.view = View::List)),
            ("scope", Box::new(|o| o.scope = None)),
            ("region", Box::new(|o| o.keyboard.region = Region::List)),
            ("field", Box::new(|o| o.keyboard.field = None)),
            ("typing", Box::new(|o| o.keyboard.typing = false)),
            ("reachable", Box::new(|o| o.keyboard.reachable = false)),
            ("cursor.index", Box::new(|o| o.cursor.index = Some(3))),
            ("cursor.id", Box::new(|o| o.cursor.id = None)),
            ("cursor.subject", Box::new(|o| o.cursor.subject = None)),
            (
                "rows.first_visible",
                Box::new(|o| o.rows.first_visible = Some(1)),
            ),
            ("rows.count", Box::new(|o| o.rows.count = Some(41))),
            ("selection.count", Box::new(|o| o.selection.count = 0)),
            ("overlay.kind", Box::new(|o| o.overlay.kind = Overlay::None)),
            ("overlay.mode", Box::new(|o| o.overlay.mode = None)),
            ("notice.text", Box::new(|o| o.notice.text = None)),
            (
                "notice.tone",
                Box::new(|o| o.notice.tone = Some(Tone::Error)),
            ),
            ("notice.undo", Box::new(|o| o.notice.undo = false)),
            ("banner.title", Box::new(|o| o.banner.title = None)),
            ("reading.id", Box::new(|o| o.reading.id = None)),
            ("reading.focused", Box::new(|o| o.reading.focused = None)),
            ("reading.scroll", Box::new(|o| o.reading.scroll = None)),
            ("composer.open", Box::new(|o| o.composer.open = false)),
            (
                "composer.detached",
                Box::new(|o| o.composer.detached = true),
            ),
            ("back_depth", Box::new(|o| o.back_depth = None)),
        ];
        for (name, change) in changes {
            let mut b = full();
            change(&mut b);
            assert!(!a.shared_eq(&b), "{name} must count");
        }
    }

    #[test]
    fn enums_serialise_to_their_snake_case_names() {
        let s = |v: &dyn AsJson| v.json();
        assert_eq!(s(&View::FirstRun), "\"first_run\"");
        assert_eq!(s(&Region::Cheatsheet), "\"cheatsheet\"");
        assert_eq!(s(&Overlay::Keymap), "\"keymap\"");
        assert_eq!(s(&Tone::Warning), "\"warning\"");
        assert_eq!(s(&Window::Closed), "\"closed\"");
        assert_eq!(s(&Region::None), "\"none\"");
    }

    trait AsJson {
        fn json(&self) -> String;
    }
    impl<T: Serialize> AsJson for T {
        fn json(&self) -> String {
            serde_json::to_string(self).unwrap()
        }
    }
}
