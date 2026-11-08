import streamDeck from "@elgato/streamdeck";
import { seam } from "./seam.js";
import { LineUp, NextZone, PreviousZone, Remap, Desk, Zones } from "./actions/simple.js";
import { SwitchWorkspace } from "./actions/workspace.js";

streamDeck.logger.setLevel("info");

streamDeck.actions.registerAction(new SwitchWorkspace());
streamDeck.actions.registerAction(new LineUp());
streamDeck.actions.registerAction(new Remap());
streamDeck.actions.registerAction(new Zones());
streamDeck.actions.registerAction(new NextZone());
streamDeck.actions.registerAction(new PreviousZone());
streamDeck.actions.registerAction(new Desk());

// An error nobody caught must not take the plugin, and so every key, down with it.
process.on("uncaughtException", (error) => streamDeck.logger.error(`Uncaught: ${error.message}`));
process.on("unhandledRejection", (reason) => streamDeck.logger.error(`Unhandled: ${String(reason)}`));

seam.start();
await streamDeck.connect();
