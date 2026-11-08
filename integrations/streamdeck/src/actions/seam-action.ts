import streamDeck, {
	type DidReceiveSettingsEvent,
	type KeyAction,
	type KeyDownEvent,
	type WillAppearEvent,
	type WillDisappearEvent,
	SingletonAction,
} from "@elgato/streamdeck";
import type { JsonObject } from "@elgato/utils";
import { seam, type SeamState, type Status } from "../seam.js";

const OFFLINE_TITLE: Record<Exclude<Status, "online">, string> = {
	offline: "Seam isn't\nrunning",
	blocked: "Other apps\nare off\nin Seam",
};

/**
 * What every Seam key does the same way: repaint when Seam changes, say so on
 * the key while Seam is away, and turn a failed press into an alert.
 */
export abstract class SeamAction<T extends JsonObject = JsonObject> extends SingletonAction<T> {
	readonly #settings = new Map<string, T>();

	constructor() {
		super();
		seam.onChange(() => this.refresh());
	}

	/** Draw the key for a connected Seam. */
	protected abstract render(key: KeyAction<T>, settings: T, state: SeamState): Promise<void>;

	/** What a press does. Throw to show an alert. */
	protected abstract press(key: KeyAction<T>, settings: T, state: SeamState): Promise<void>;

	protected remember(id: string, settings: T): void {
		this.#settings.set(id, settings);
	}

	protected refresh(): void {
		for (const action of this.actions) {
			if (action.isKey()) {
				void this.paint(action);
			}
		}
	}

	protected async paint(key: KeyAction<T>): Promise<void> {
		try {
			const state = seam.state;
			if (seam.status !== "online" || !state) {
				await key.setState(0);
				await key.setTitle(OFFLINE_TITLE[seam.status === "blocked" ? "blocked" : "offline"]);
				return;
			}
			await key.setTitle(undefined);
			await this.render(key, this.#settings.get(key.id) ?? ({} as T), state);
		} catch (error) {
			streamDeck.logger.warn(`Couldn't draw a key: ${(error as Error).message}`);
		}
	}

	override async onWillAppear(ev: WillAppearEvent<T>): Promise<void> {
		this.remember(ev.action.id, ev.payload.settings);
		if (ev.action.isKey()) {
			await this.paint(ev.action);
		}
	}

	override async onDidReceiveSettings(ev: DidReceiveSettingsEvent<T>): Promise<void> {
		this.remember(ev.action.id, ev.payload.settings);
		if (ev.action.isKey()) {
			await this.paint(ev.action);
		}
	}

	override onWillDisappear(ev: WillDisappearEvent<T>): void {
		this.#settings.delete(ev.action.id);
	}

	override async onKeyDown(ev: KeyDownEvent<T>): Promise<void> {
		const key = ev.action;
		if (!key.isKey()) {
			return;
		}
		this.remember(key.id, ev.payload.settings);
		const state = seam.state;
		try {
			if (seam.status !== "online" || !state) {
				await key.showAlert();
				return;
			}
			await this.press(key, ev.payload.settings, state);
		} catch (error) {
			streamDeck.logger.warn(`${this.manifestId} failed: ${(error as Error).message}`);
			await key.showAlert();
		}
		// Stream Deck flips a two-state key on every press. Seam has the final say.
		await this.paint(key);
	}
}
