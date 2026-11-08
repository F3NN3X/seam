import streamDeck from "@elgato/streamdeck";
import { seam } from "./seam.js";
import {
	LineUp,
	NextZone,
	PreviousZone,
	PinOnTop,
	ReleasePinned,
	Remap,
	Desk,
	SnapBack,
	Zones,
} from "./actions/simple.js";
import { FocusWindow, MoveWindow } from "./actions/window.js";
import { SwitchWorkspace } from "./actions/workspace.js";

streamDeck.logger.setLevel("info");

streamDeck.actions.registerAction(new SwitchWorkspace());
streamDeck.actions.registerAction(new SnapBack());
streamDeck.actions.registerAction(new LineUp());
streamDeck.actions.registerAction(new Remap());
streamDeck.actions.registerAction(new Zones());
streamDeck.actions.registerAction(new NextZone());
streamDeck.actions.registerAction(new PreviousZone());
streamDeck.actions.registerAction(new MoveWindow());
streamDeck.actions.registerAction(new FocusWindow());
streamDeck.actions.registerAction(new PinOnTop());
streamDeck.actions.registerAction(new ReleasePinned());
streamDeck.actions.registerAction(new Desk());

// An error nobody caught must not take the plugin, and so every key, down with it.
process.on("uncaughtException", (error) => streamDeck.logger.error(`Uncaught: ${error.message}`));
process.on("unhandledRejection", (reason) => streamDeck.logger.error(`Unhandled: ${String(reason)}`));

seam.start();
await streamDeck.connect();
