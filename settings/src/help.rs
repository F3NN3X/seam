//! What the Help page says, kept as data so search can find it. `**` marks the
//! exact label of something in Seam, which shows in bold.

use crate::panel::Page;

pub enum Block {
    Text(&'static str),
    List(&'static [&'static str]),
    /// A step, with a button that opens the page that does it.
    Step {
        title: &'static str,
        text: &'static str,
        page: Page,
    },
}

pub struct Topic {
    pub title: &'static str,
    pub blocks: &'static [Block],
    /// The page the button under the text opens.
    pub opens: Option<Page>,
}

impl Topic {
    /// Whether its title or text holds `query`, which is already lower case.
    pub fn mentions(&self, query: &str) -> bool {
        let has = |text: &str| plain(text).to_lowercase().contains(query);
        has(self.title)
            || self.blocks.iter().any(|block| match block {
                Block::Text(text) => has(text),
                Block::List(items) => items.iter().any(|item| has(item)),
                Block::Step { title, text, .. } => has(title) || has(text),
            })
    }
}

/// The text without its bold marks.
pub fn plain(text: &str) -> String {
    text.replace("**", "")
}

use Block::{List, Step, Text};

pub const TOPICS: &[Topic] = &[
    Topic {
        title: "Getting started",
        blocks: &[
            Text(
                "Seam makes the pointer cross where your screens physically meet, and puts your windows where you want them. Three steps get you there.",
            ),
            Step {
                title: "Line up your screens",
                text: "Drag two lines across each border until the stripes run straight over the bezel",
                page: Page::Displays,
            },
            Step {
                title: "Pick zone layouts",
                text: "Choose Columns, Rows, Grid, Priority, Main + stack or Stack + main for each screen",
                page: Page::Zones,
            },
            Step {
                title: "Capture a workspace",
                text: "Arrange your windows, then save which app is where",
                page: Page::Workspaces,
            },
        ],
        opens: None,
    },
    Topic {
        title: "Pointer and borders",
        blocks: &[
            Text(
                "Windows keeps the same pixel row when the pointer crosses to another screen, so between screens of different sizes it jumps. Seam puts it where the screens physically meet instead.",
            ),
            Text(
                "Press **Line up screens** on the Displays page, or Ctrl+Shift+Alt+L. Every screen shows its number, and for each border:",
            ),
            List(&[
                "Grab a line to move just that line. This sets the scale.",
                "Drag anywhere else to move both lines. This sets the height.",
                "Arrow keys nudge the selected line, Tab switches lines and R goes back to the guess from your screens' sizes",
                "Enter goes to the next border or finishes, Backspace goes back and Esc cancels",
            ]),
            Text(
                "When you finish, Seam saves the borders and turns on **Remap pointer at borders**. Ctrl+Shift+Alt+P pauses and resumes it. With **Also align Windows arrangement** on, Seam also moves your screens in Display settings to match.",
            ),
            Text(
                "Each set of screens is a desk, with its own borders, zones and workspaces. **Saved desks** lists them, with when each was lined up, in your local time. Each desk's menu has **Rename** and **Forget**.",
            ),
            Text(
                "**Edge resistance**, on Displays, holds the pointer at the edge to another screen until you push that many pixels past it, so you can grab a window's edge or corner there. It's off at 0, works at every border, and gives way while a mouse button is down, so dragging a window across is never slowed. It stays off while the remap is paused and during Line Up.",
            ),
            Text(
                "Seam relies on Windows' **Ease cursor movement between displays**, so the pointer can always leave a screen toward its neighbour. When it's off, the Displays page says so, and **Turn it on** turns it on for you and checks that Windows kept it. Seam never turns it off.",
            ),
        ],
        opens: Some(Page::Displays),
    },
    Topic {
        title: "Line Up aids",
        blocks: &[
            Text(
                "Each step names both screens with their models, such as \"Line up screen 3 (HDMI) with screen 4 (U3477)\", and says which way to move each line, such as straight below or level with the line on the reference screen.",
            ),
            List(&[
                "A band of faint stripes runs along the shared edge on both screens, 10 mm apart on your desk. If they run on straight across the edge, the screens are lined up. If they step, the offset is off, and if the spacing changes, the scale is off.",
                "A screen that reports no size gets no stripes",
                "With **Type measurements in Line Up** on, on the Displays page, M types the offset instead of dragging. It asks how many cm one screen's top or left edge is below or to the right of the other's, and moves the lines to match. Tab flips the direction, Enter applies and Esc closes the field.",
                "Typing needs both screens to report their size",
            ]),
        ],
        opens: Some(Page::Displays),
    },
    Topic {
        title: "Glue and Not on my desk",
        blocks: &[
            Text("Windows doesn't always have your screens as they stand on your desk. Line Up handles two cases."),
            Text(
                "**Glue** is for screens that touch on your desk but have a gap in Windows. Line Up offers each pair after the touching borders. Press G to glue it, then line the two up as they stand on your desk. The gap doesn't matter. A pair found from the screens' sizes starts glued, and a pair found only from the pixel layout starts off, so Enter skips it.",
            ),
            List(&[
                "The pointer rests at a glued edge until you push past it, 60 px unless you change **Glued edge push** on Displays, so an auto-hide taskbar there still shows. A quick flick crosses at once.",
                "Past the end of a glue, the edge is a dead edge, and Seam holds the pointer there",
                "**Align arrangement** keeps glued screens apart, and refuses a move that would break a glue",
            ]),
            Text(
                "**Not on my desk** is for two screens that touch in Windows but not on your desk. Press N in Line Up, or choose **Not on my desk** in the border's **...** menu on Displays. The pointer stops at that edge both ways, or crosses to a glued screen past it. **On my desk** takes it back.",
            ),
            Text("Displays describes these borders as glued or \"Not on your desk\"."),
        ],
        opens: Some(Page::Displays),
    },
    Topic {
        title: "Layout views",
        blocks: &[
            Text(
                "The **Physical layout** card on Displays shows your screens two ways. Switch between them above the map.",
            ),
            List(&[
                "**Seam** is your desk, at the screens' real sizes. A lined up border is a solid line, a glued one is dashed and one that isn't on your desk is dotted.",
                "**Windows** is the arrangement as Windows has it, in pixels. A gap shows a note such as \"18 px gap\".",
            ]),
            Text(
                "Click a screen to flash its number on it. **Show on screens** draws the chosen view on every real screen for a few seconds, with each screen's number and model and a band along each edge that has a neighbour. Esc or a click closes it.",
            ),
        ],
        opens: Some(Page::Displays),
    },
    Topic {
        title: "Zones",
        blocks: &[
            Text(
                "Pick a layout for each screen: Columns, Rows, Grid, Priority, Main + stack or Stack + main. Then hold Shift while you drag a window. The zones show on the screen under the pointer, and letting go drops the window into the one that's lit.",
            ),
            List(&[
                "Hold Ctrl as well to cover several zones with one window",
                "**Show zones** and **Span zones** change the two keys",
                "**Gap** sets the space between zones and the screen edge",
                "**Restore size when dragged out** gives a window its old size back when you drop it outside a zone",
                "**Swap with the window already in a zone** makes a window you drop on a held zone trade places with it; off lets them overlap",
            ]),
            Text(
                "While zones show, the lit zone is where the window will land, and thin grey outlines show where any window it pushes aside will go. Until you let go:",
            ),
            List(&[
                "**Scroll the mouse wheel** to step through the whole zone, 1/2, 1/3, 2/3, the zone with its neighbour, and the whole screen. It stops at either end. A share sits on the side of the zone nearest the pointer and follows it",
                "**Right-click** drops the window where you let go, without a zone",
                "**Tab** steps through the zone, the zone with its neighbour on the side you lean to, and the whole screen. **Shift+Tab** goes back, unless Shift is your show key",
                "**1**, **2** or **3** give the window a half, a third or two thirds of it, and **0** all of it. The rest becomes a zone of its own, and a window that was there moves into it. The layout is saved as **Changed**, and a notification offers Undo for 8 seconds",
                "The **arrows** pick the side the part sits on, until the pointer moves to another zone",
                "**Esc** drops the window where you let go, without a zone",
            ]),
            Text(
                "Other keys, clicks and moves reach your apps as usual. **Pause to snap** shows the zones when you hold the pointer still for a moment while dragging, with no key. **Show keys while dragging** puts a strip on the screen that names the wheel, right-click and these keys.",
            ),
            Text(
                "Drag the edge of a window in a zone and the windows in the zones on the other side follow it. No window goes below the smallest size its app allows: the edge beyond it moves instead. When you let go, the window fits its zone again and the screen's layout shows as **Changed**. Esc puts everything back. **Resize neighbours with a window** turns this off.",
            ),
            Text(
                "Between two snapped windows, the gap has a handle you can't see until the pointer is on it, then a slim bar shows. Drag it and the windows on both sides follow, with the same smallest sizes, and the layout is saved as **Changed** when you let go. Esc puts everything back. Right-click it to give the left or top window 1/2, 1/3, 2/3, 1/4 or 3/4 of the two, or to take one of them out: it leaves its zones with its old size, and the others stay. A gap under 4 px has no handles, so **Size menu** on Shortcuts opens the same choices for the focused window. It has no key until you set one. **Handles between snapped windows** turns the handles off.",
            ),
            Text(
                "**Edit layout** opens the editor on the screen itself. Click a zone to split it side by side or top and bottom, drag a divider, or merge two neighbours, then press **Done**. Esc throws the changes away and Ctrl+Z undoes the last one.",
            ),
            Text(
                "A layout you make is for the screen you made it on, and that screen's tab shows it under **Your layouts**. **Used on...** in its **...** menu ties it to other screens or to any screen, and **Show all screens** lists every layout, each tied one saying which screen it's for.",
            ),
            Text(
                "Your own layouts are kept in a library that every desk and screen shares, each with a tile under the built-in ones. Click one to give it to the screen: the windows on it move into the new zones, one to a zone and snapped ones first, and when any moved, a notification offers Undo for 8 seconds. Or use its **...** menu to edit, rename, duplicate or delete it. Editing one changes every screen that uses it, and deleting one leaves those screens with their zones. **New layout** starts one from a single zone, and **Edit layout** on a template saves what you make as a new one, leaving the template as it was.",
            ),
            Text(
                "**Save from windows** makes a library layout from where the windows on the screen are now, a zone for each, and snaps them in. They need to line up: no overlaps, and straight lines across the screen that separate them. A layout that is none of the tiles shows as **Changed**, and **Save as layout** keeps it in the library.",
            ),
            Text(
                "**Keep windows in their zones** is experimental. It moves snapped windows back after a screen, resolution or scaling change, or after the PC wakes.",
            ),
            Text(
                "**Tile this screen's windows** puts the loose windows on a screen into its zones in one go: the one you are using gets the biggest zone, and the others take the nearest free one. A screen with no layout is given one that fits the number of windows, from two columns up to a 3x3 grid. It takes up to nine windows and leaves pinned, sticky, minimized and administrator windows alone. Tile windows in the tray menu does it for a screen too, and a notification offers Undo for 8 seconds. The shortcut has no key until you set one.",
            ),
            Text(
                "**Quick layout** opens a small panel on the screen under the pointer with that screen's layouts as numbered thumbnails: the built-in ones, the custom layouts made for it, then None. Press 1 to 9 to pick one, or use the arrows and Enter (the arrows scroll past nine). Ctrl+Z undoes the last pick made there, Esc or a click outside closes it, and the panel stays open after a pick so you can try another. A pick moves the windows on that screen as clicking a tile does. With zones off the panel says so, and a pick is still saved. The shortcut has no key until you set one.",
            ),
            Text("Layouts belong to the desk, so each set of screens keeps its own."),
        ],
        opens: Some(Page::Zones),
    },
    Topic {
        title: "Moving windows",
        blocks: &[
            Text("Left means left on your desk, as the screens physically stand, not in Windows' pixel layout."),
            List(&[
                "Shift+Win+Alt+arrows move the focused window. A window in a zone goes to the next zone that way, then onto the screen next to it.",
                "Ctrl+Shift+Alt+arrows focus the nearest window that way",
                "Swap trades places with the nearest window that way. It has no key until you set one.",
            ]),
            Text(
                "A window in a zone can take room from its neighbours. **Grow window into its neighbour** and **Shrink window** move the divider beside it, towards the bigger neighbour first. **Give window two thirds of its group** makes it the main window on its screen, and pressing it again puts the zones back. The windows beside it follow, and no window goes below the smallest size its app allows. A changed layout shows as Custom on the Zones page. These have no key until you set one.",
            ),
            Text(
                "**When a window moves to another screen**, on the Zones page, sets the size a window gets there: **Let Windows scale it**, **Keep its pixel size** or **Keep its real size**. **Also when I drag a window to another screen** does the same for a window you drop with the mouse.",
            ),
            Text(
                "Seam leaves alone the windows of apps running as administrator, windows that aren't responding, and its own.",
            ),
            Text(
                "If komorebi, GlazeWM or LeopardWM is running, Seam leaves placing windows to it. **Another window manager handles my windows** does the same for any other one.",
            ),
        ],
        opens: Some(Page::Shortcuts),
    },
    Topic {
        title: "Focus and spotlight",
        blocks: &[
            List(&[
                "**Focus on this window** minimizes the other windows on its screen, and pressing it again brings them back. Windows pinned on top or kept out of workspace switches stay, and so do the zones' excluded apps.",
                "**Spotlight** dims everything but the window you're using, on every screen. Press it again to turn it off. With the desktop or taskbar in front, the dim is hidden.",
            ]),
            Text(
                "Neither has a key until you set one on the Shortcuts page. Focus on this window leaves placing windows to another window manager if one is running, and Spotlight keeps working next to it.",
            ),
        ],
        opens: Some(Page::Shortcuts),
    },
    Topic {
        title: "Apps running as administrator",
        blocks: &[
            Text(
                "Windows doesn't let an app move the windows of an app that runs with more rights. That's why an elevated app's window can't move with Seam.",
            ),
            List(&[
                "To manage them, quit Seam and start it with Run as administrator. Seam then manages both, but its Start with Windows entry still starts it normally.",
                "Or leave those windows alone. The pointer remap and Line Up work either way.",
            ]),
        ],
        opens: None,
    },
    Topic {
        title: "Pins and borders",
        blocks: &[
            List(&[
                "**Pin window on top** keeps the focused window above the others until you press it again",
                "**Keep window out of workspace switches** makes it sticky: a switch never moves, minimizes or closes it, and capture leaves it out",
                "**Release pinned windows**, in the tray menu, takes back every pin and border Seam set. Quitting Seam does the same.",
            ]),
            Text("The pin keys have no key until you set one on the Shortcuts page."),
            Text(
                "**Focus border** colours the border of the window that has focus. A pinned window gets an amber border, which wins over the focus border. Both colours are picked from the swatches on the Zones page, with **None** for no border and **Custom colour** for any other.",
            ),
            Text(
                "**Tell me when a pinned window closes** tells you after the app closes. Click the notification to start it again and pin its new window.",
            ),
            Text(
                "**Pinned windows stay when I switch workspaces** makes a switch leave a window pinned on top alone, like a sticky one.",
            ),
        ],
        opens: Some(Page::Zones),
    },
    Topic {
        title: "Workspaces",
        blocks: &[
            Text(
                "A workspace saves which app is on which screen and in which zone, how each window shows, and which one had the focus, for the desk you have connected.",
            ),
            List(&[
                "**Capture current windows** saves the windows open now. Only app windows with a taskbar button count.",
                "**Switch now**, the workspace's shortcut or **Workspaces** in the tray menu puts each app back. Windows already in place aren't touched.",
                "**Open alongside**, on a workspace's menu, switches once and leaves the other windows alone, whatever **Other windows** says",
                "**Start missing apps** starts the ticked apps that aren't running, and **Other windows** leaves, minimizes or closes the rest. Close only asks, so an app with unsaved work stays open.",
                "**Update from screen** saves the windows as they are now, and keeps your edits and the apps that aren't open, unless **Update from screen removes apps that aren't open** is on",
            ]),
            Text(
                "The table is an editor, and each change saves at once with Undo. Pick where an app goes and how it shows. The rows are the start order: drag one by its grip, or press Alt+Up and Alt+Down. Click an app's name for a delay and **Wait for this app's window before opening the next**. **Add an app** picks one from the open windows or the Start menu.",
            ),
            Text(
                "**Zones**, under the table, gives each screen the layout a switch sets before it places the apps: **Keep as is**, **No zones**, a built-in layout or one from your library. Capture and **Update from screen** record the layout each screen has. A library layout you edit changes here too.",
            ),
            Text(
                "The **Now** column shows how each app is doing. When something moved or was minimized, or a screen's layout isn't the workspace's, **Put windows back** puts it back. That's Snap back, which also has a shortcut, **Snap back to this workspace**.",
            ),
            Text(
                "**Switch back to the previous workspace** is a shortcut, and the first item of **Workspaces** in the tray menu. It goes to the workspace that was in use before this one, and pressing it again goes back.",
            ),
            Text(
                "**Put apps in place when they open** sends an app of the workspace in use to its place once, a moment after you open it yourself.",
            ),
            Text(
                "**Remember folders and pages** is off until you turn it on. Then a capture also saves the folder each Explorer window shows and the page each browser shows, and a switch that starts the app opens it there. An app that's already open is never sent anywhere. Only folders on this PC's own drives (not mapped ones) and http or https pages are kept. Private windows are skipped when their title says so. Export leaves them out, and import drops them. With it off, nothing is read, and **Forget saved folders and pages** removes the ones saved.",
            ),
            Text(
                "**Update from screen removes apps that aren't open** is off until you turn it on. Then **Update from screen** removes the apps with no window open instead of keeping them, and a notification offers Undo for 8 seconds, which puts them back in the same place, as long as you haven't changed the workspace since. An app stays when it has a window Seam didn't take, such as on another desktop, off every screen or already held by another row (a suspended app's window can count too), and nothing is removed when no app of the workspace is open. **Save windows to the active workspace** keeps them either way.",
            ),
            Text(
                "**Save windows to the active workspace** is a shortcut that saves into the workspace in use without opening Seam. Click its notification within 8 seconds to undo it.",
            ),
            Text(
                "The workspace in use shows as Active in the list, and as Active, moved once one of its windows moved. **Set as default**, on a workspace's menu, makes it the one in use when Seam starts and none is remembered.",
            ),
        ],
        opens: Some(Page::Workspaces),
    },
    Topic {
        title: "Where apps open",
        blocks: &[
            Text(
                "**Where apps open**, on the Zones page, sends an app's window to a place whenever it opens, with or without a workspace. **Add an app**, then pick its screen and zone, or the full screen.",
            ),
            Text(
                "**Put apps in place when they open** on Workspaces handles the workspace in use: a window goes where that workspace's row for the app says. An app rule is for any other time. When the workspace in use has a row for an app, it owns every window of that app and the rule never applies to it.",
            ),
            Text(
                "A rule moves a window once, a moment after it opens. Dialogs, windows that open while a workspace switches, apps that run as administrator and apps under **Left alone** stay where they open, and so does a window whose rule's screen isn't connected.",
            ),
            Text(
                "**...**, **Keep on top** on an app's row pins its windows as you would by hand, once, a moment after they open. Choose **Doesn't move** as its place to only pin it. **Release pinned windows** takes the pin back, and the rule leaves that window alone until it's opened again.",
            ),
            Text("**Left alone** lists the apps Seam never manages. Remove an app there to manage it again."),
        ],
        opens: Some(Page::Zones),
    },
    Topic {
        title: "When you sign in",
        blocks: &[
            Text(
                "**When I sign in**, on General, picks what Seam does on this desk when it starts at sign-in: **Do nothing** (the default), **Offer my last session** or **Open a workspace**. Starting Seam by hand does none of them.",
            ),
            Text(
                "**Offer my last session** keeps a hidden last session for each desk: which apps are where, and which are pinned. At sign-in Seam asks whether to restore it. A click puts the windows back in their zones and pins them again. Nothing starts without the click.",
            ),
            Text(
                "**Open a workspace** switches to the workspace you pick, like any workspace switch. **Ask first** is on by default: Seam shows \"Open <workspace>? Click to open\" and waits for the click. Off, it switches about 10 seconds after the taskbar is up. If a full-screen app is in front it waits, up to 3 minutes after Seam starts. A pause by hand cancels it. It acts on the desk that is current then.",
            ),
            Text(
                "**Who reopens the apps** picks **Windows reopens them**, where Seam puts each window back as it appears, or **Seam reopens them**, where Seam starts the missing apps itself.",
            ),
        ],
        opens: Some(Page::General),
    },
    Topic {
        title: "Pause and full-screen apps",
        blocks: &[
            Text(
                "**Step aside for games and full-screen apps**, on General, is on by default. While a game, a video or a slide show fills a screen, Seam gives its shortcuts back to that app, shows no zones when you drag, and turns Spotlight and the focus border off. Everything comes back when you leave it.",
            ),
            Text(
                "**Pause Seam**, in the tray menu or on a shortcut you set on Shortcuts, does the same by hand and also turns the pointer remap off and leaves new windows where they open. It isn't saved, so Seam is running again after a restart.",
            ),
        ],
        opens: Some(Page::General),
    },
    Topic {
        title: "Shortcuts",
        blocks: &[
            Text("Shortcuts work anywhere in Windows, even with the Seam window closed."),
            List(&[
                "Ctrl+Shift+Alt+L lines up your screens",
                "Ctrl+Shift+Alt+P pauses or resumes the remap",
                "Pause Seam has no key until you set one, and pauses or resumes everything",
                "Shift+Win+Alt+arrows move the focused window, and Ctrl+Shift+Alt+arrows focus the window that way",
                "Swap, grow, shrink, two thirds, tile, next and previous zone, the pins, capture, Snap back, Switch back, quick save, Focus on this window and Spotlight have no key until you set one",
            ]),
            Text(
                "Each workspace can have its own shortcut too. Press **Change** on a row, then the keys. Esc keeps the old ones and Backspace clears them. Ctrl+Shift+Alt plus a number is a good choice, since Ctrl+Alt alone types characters on many keyboards.",
            ),
            Text("If another app already holds a shortcut, its keys turn red and Seam keeps running without it."),
        ],
        opens: Some(Page::Shortcuts),
    },
    Topic {
        title: "Stream Deck",
        blocks: &[
            Text(
                "The Seam plugin puts Seam on Stream Deck keys. It talks to Seam through the local API, so **Let other apps control Seam** on General has to stay on.",
            ),
            List(&[
                "Switch workspace, Snap back and Save windows",
                "Line up screens, Pause or resume remap, and Zones on or off",
                "Next zone, Previous zone, Move window and Focus window",
                "Pin on top, Release pinned windows, and Desk, which shows the desk's name",
            ]),
            Text(
                "To install it, open the co.f3nn3x.seam.streamDeckPlugin file, and the Stream Deck app adds a Seam group to its actions. When Seam isn't running, every key says so and reconnects by itself.",
            ),
        ],
        opens: Some(Page::General),
    },
    Topic {
        title: "Local API",
        blocks: &[
            Text(
                "Tools on this PC can control Seam through its local API, the way the Stream Deck plugin does. They can read status, switch workspaces, line up screens and move windows.",
            ),
            Text(
                "**Let other apps control Seam**, on General, turns it on or off, and says how many apps are connected. The Seam window has its own connection, so it keeps working either way.",
            ),
        ],
        opens: Some(Page::General),
    },
    Topic {
        title: "Updates",
        blocks: &[
            Text("About shows your version and the result of each update check."),
            List(&[
                "**Check for updates every day** looks once a day, and **Check now** looks straight away",
                "**Install updates on their own** installs only while you're away from the computer",
                "**Include pre-releases** gets new features early. They may have rough edges.",
            ]),
            Text(
                "The updater only installs releases signed with Seam's key. The Store copy is updated by the Microsoft Store.",
            ),
        ],
        opens: Some(Page::About),
    },
];
