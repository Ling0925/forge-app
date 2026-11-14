use std::collections::BTreeMap;

pub const SEND_BEFORE_COMMIT: &str = "send.before_commit";
pub const SEND_AFTER_COMMIT: &str = "send.after_commit";
pub const TOOL_AFTER_SPAWN: &str = "tool.after_spawn";
pub const TOOL_AFTER_SIDE_EFFECT_BEFORE_COMMIT: &str = "tool.after_side_effect_before_commit";
pub const RUN_BEFORE_FINAL_COMMIT: &str = "run.before_final_commit";

#[derive(Debug, PartialEq, Eq)]
pub struct InjectedCrash {
    pub point: String,
}

#[derive(Clone, Copy)]
pub enum Action {
    Panic,
    Abort,
}

#[derive(Default, Clone)]
pub struct Failpoints {
    enabled: BTreeMap<String, Action>,
}

impl Failpoints {
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            enabled: names
                .into_iter()
                .map(|name| (name.into(), Action::Panic))
                .collect(),
        }
    }

    pub fn with_actions(points: impl IntoIterator<Item = (impl Into<String>, Action)>) -> Self {
        Self {
            enabled: points
                .into_iter()
                .map(|(name, action)| (name.into(), action))
                .collect(),
        }
    }

    pub fn hit(&self, name: &str) {
        match self.enabled.get(name) {
            Some(Action::Panic) => std::panic::panic_any(InjectedCrash {
                point: name.to_owned(),
            }),
            Some(Action::Abort) => std::process::abort(),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_named_point_panics_only_when_configured() {
        for name in [
            SEND_BEFORE_COMMIT,
            SEND_AFTER_COMMIT,
            TOOL_AFTER_SPAWN,
            TOOL_AFTER_SIDE_EFFECT_BEFORE_COMMIT,
            RUN_BEFORE_FINAL_COMMIT,
        ] {
            let points = Failpoints::new([name]);
            crate::failpoint!(points, "unconfigured");
            let crash = std::panic::catch_unwind(|| crate::failpoint!(points, name)).unwrap_err();
            assert_eq!(
                *crash.downcast::<InjectedCrash>().unwrap(),
                InjectedCrash { point: name.into() }
            );
            crate::failpoint!(Failpoints::default(), name);
        }
    }
}
