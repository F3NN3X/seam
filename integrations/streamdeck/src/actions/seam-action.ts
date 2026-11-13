import streamDeck, {
	type DidReceiveSettingsEvent,
	type KeyAction,
	type KeyDownEvent,
	type TitleParametersDidChangeEvent,
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

	protected settingsOf(id: string): T {
		return this.#settings.get(id) ?? ({} as T);
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
			await this.render(key, this.settingsOf(key.id), state);
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

/**
 * A key that puts a title of its own on itself unless the user typed one. Stream Deck
 * reports the title on screen, ours included. A title is the user's only when it differs
 * from the last one this key was given by us and isn't a manifest default. With no record
 * of ours (the plugin restarted), it is ours if it is what the key would show now.
 */
export abstract class TitledAction<T extends JsonObject = JsonObject> extends SeamAction<T> {
	readonly #typed = new Set<string>();
	/** Per key, the last title we sent: undefined is the user's or the manifest's. */
	readonly #last = new Map<string, string | undefined>();
	readonly #manifest: ReadonlySet<string>;

	/** `manifestTitles` are the titles the manifest gives the key's states. */
	constructor(manifestTitles: string[]) {
		super();
		this.#manifest = new Set(manifestTitles);
	}

	/** The title the key shows with the settings and state it has, or undefined for none. */
	protected abstract titleFor(settings: T, state: SeamState): string | undefined;

	/** Show `title`, or leave the user's if they typed one. */
	protected async titleKey(key: KeyAction<T>, title: string | undefined): Promise<void> {
		const shown = this.#typed.has(key.id) ? undefined : title;
		this.#last.set(key.id, shown);
		await key.setTitle(shown);
	}

	override async onTitleParametersDidChange(ev: TitleParametersDidChangeEvent<T>): Promise<void> {
		const { title } = ev.payload;
		const id = ev.action.id;
		let typed = false;
		if (title !== "" && !this.#manifest.has(title)) {
			if (this.#last.has(id)) {
				typed = title !== this.#last.get(id);
			} else {
				const state = seam.state;
				typed = !state || title !== this.titleFor(this.settingsOf(id), state);
				if (!typed) {
					this.#last.set(id, title);
				}
			}
		}
		if (typed === this.#typed.has(id)) {
			return;
		}
		if (typed) {
			this.#typed.add(id);
		} else {
			this.#typed.delete(id);
		}
		if (ev.action.isKey()) {
			await this.paint(ev.action);
		}
	}

	override onWillDisappear(ev: WillDisappearEvent<T>): void {
		this.#typed.delete(ev.action.id);
		this.#last.delete(ev.action.id);
		super.onWillDisappear(ev);
	}
}
