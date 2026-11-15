//! The tools an assistant sees, and the API call each one makes.

use serde_json::{Map, Value, json};

/// What a tool does to the desk, which MCP clients show as hints.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Read,
    /// Changes windows or workspaces, but can be undone and closes nothing.
    Act,
}

pub struct Tool {
    pub name: &'static str,
    pub method: &'static str,
    pub about: &'static str,
    pub kind: Kind,
    /// Argument name, JSON type, and what it is. Every one is optional unless in `required`.
    pub args: &'static [(&'static str, &'static str, &'static str)],
    pub required: &'static [&'static str],
}

const ID: (&str, &str, &str) = ("id", "string", "The workspace id, as seam_workspaces lists it");
const ROW: (&str, &str, &str) = ("row", "integer", "The row number, from 0, as seam_workspace lists it");

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "seam_state",
        method: "state.get",
        about: "What Seam is doing now: the desk and its screens, the workspace in use and whether its windows have drifted, and whether Seam is paused.",
        kind: Kind::Read,
        args: &[],
        required: &[],
    },
    Tool {
        name: "seam_workspaces",
        method: "workspaces.list",
        about: "The saved workspaces of the current desk: id, name, shortcut, how many apps each holds, and which is in use.",
        kind: Kind::Read,
        args: &[],
        required: &[],
    },
    Tool {
        name: "seam_workspace",
        method: "workspace.get",
        about: "The apps of one workspace, row by row: the app's name, window title, screen, where it goes, how it is shown, and whether Seam starts it.",
        kind: Kind::Read,
        args: &[ID],
        required: &["id"],
    },
    Tool {
        name: "seam_workspace_status",
        method: "workspace.status",
        about: "How each app of a workspace is doing right now: in_place, moved, minimized, not_running, background or cant_move.",
        kind: Kind::Read,
        args: &[ID],
        required: &["id"],
    },
    Tool {
        name: "seam_switch_workspace",
        method: "workspace.switch",
        about: "Switch to a workspace by name or id. Windows it doesn't mention are left alone and nothing is closed, but apps it lists that aren't running may be started.",
        kind: Kind::Act,
        args: &[("name", "string", "The workspace's name, matched without regard to case"), ID],
        required: &[],
    },
    Tool {
        name: "seam_snap_back",
        method: "workspace.snap_back",
        about: "Put the windows of the workspace in use back where it says they go.",
        kind: Kind::Act,
        args: &[],
        required: &[],
    },
    Tool {
        name: "seam_switch_back",
        method: "workspace.switch_back",
        about: "Switch back to the workspace that was in use before this one.",
        kind: Kind::Act,
        args: &[],
        required: &[],
    },
    Tool {
        name: "seam_quick_save",
        method: "workspace.quick_save",
        about: "Save the windows on screen into the workspace in use, or into a new workspace if none is in use. The user can undo it from Seam's notice for a few seconds.",
        kind: Kind::Act,
        args: &[],
        required: &[],
    },
    Tool {
        name: "seam_create_workspace",
        method: "workspace.create",
        about: "Create an empty workspace on the current desk. Needs \"Let assistants change workspaces\" in Seam. The user can undo it from Seam's notice.",
        kind: Kind::Act,
        args: &[("name", "string", "The new workspace's name")],
        required: &["name"],
    },
    Tool {
        name: "seam_rename_workspace",
        method: "workspace.rename",
        about: "Rename a workspace. Needs \"Let assistants change workspaces\" in Seam. The user can undo it from Seam's notice.",
        kind: Kind::Act,
        args: &[ID, ("name", "string", "The new name")],
        required: &["id", "name"],
    },
    Tool {
        name: "seam_edit_row",
        method: "workspace.edit_row",
        about: "Change where one app of a workspace goes (target), how it is shown (shown), or stop Seam starting it (launch false). It can't change which program the row is, and can't turn starting on. Needs \"Let assistants change workspaces\" in Seam. The user can undo it from Seam's notice.",
        kind: Kind::Act,
        args: &[
            ID,
            ROW,
            (
                "target",
                "object",
                "{\"zone\": 2}, {\"zone\": 1, \"span_to\": 2}, {\"rect\": [x, y, w, h]} as fractions of the screen, or {\"state\": \"max\"}",
            ),
            ("shown", "string", "normal, minimized, maximized or fullscreen"),
            ("launch", "boolean", "Only false is accepted: Seam stops starting this app"),
        ],
        required: &["id", "row"],
    },
    Tool {
        name: "seam_remove_row",
        method: "workspace.remove_row",
        about: "Remove one app from a workspace. Needs \"Let assistants change workspaces\" in Seam. The user can undo it from Seam's notice.",
        kind: Kind::Act,
        args: &[ID, ROW],
        required: &["id", "row"],
    },
    Tool {
        name: "seam_set_show_all",
        method: "workspace.set_show_all",
        about: "Turn \"Show every window\" on or off for a workspace: with it on, a window saved minimized is shown again on a switch. Needs \"Let assistants change workspaces\" in Seam. The user can undo it from Seam's notice.",
        kind: Kind::Act,
        args: &[ID, ("on", "boolean", "true to show every window")],
        required: &["id", "on"],
    },
];

pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

impl Tool {
    /// The entry `tools/list` gives for this tool.
    pub fn listed(&self) -> Value {
        let properties: Map<String, Value> = self
            .args
            .iter()
            .map(|(name, kind, about)| (name.to_string(), json!({ "type": kind, "description": about })))
            .collect();
        let read = self.kind == Kind::Read;
        json!({
            "name": self.name,
            "description": self.about,
            "inputSchema": { "type": "object", "properties": properties, "required": self.required },
            "annotations": { "readOnlyHint": read, "destructiveHint": false, "idempotentHint": read, "openWorldHint": false },
        })
    }

    /// The API params for the arguments given. Only the tool's own arguments pass, so nothing
    /// else an assistant adds reaches Seam, and a switch is always alongside.
    pub fn params(&self, arguments: &Map<String, Value>) -> Value {
        let mut params: Map<String, Value> = self
            .args
            .iter()
            .filter_map(|(name, ..)| arguments.get(*name).map(|v| (name.to_string(), v.clone())))
            .collect();
        if self.method == "workspace.switch" {
            params.insert("alongside".into(), Value::Bool(true));
        }
        Value::Object(params)
    }
}
