import streamDeck, {
	action,
	type DidReceiveSettingsEvent,
	type KeyAction,
	type SendToPluginEvent,
} from "@elgato/streamdeck";
import type { JsonValue } from "@elgato/utils";
import { NOT_FOUND, REFUSED, seam, SeamError, type ScreenLayouts, type SeamState } from "../seam.js";
import { TitledAction } from "./seam-action.js";

/** `screen` is a Line Up number, or "pointer" (also when unset) for the screen under the pointer. */
type Settings = { screen?: string; id?: string; name?: string };

/** The Line Up number a key is set to, or undefined for the screen under the pointer. */
function screenNumber(settings: Settings): number | undefined {
	const number = Number(settings.screen);
	return Number.isInteger(number) && number > 0 ? number : undefined;
}

/** The layout a key is set to among a screen's: by id, else by the name it saved. */
function chosen(settings: Settings, offered: ScreenLayouts): ScreenLayouts["layouts"][number] | undefined {
	const name = settings.name?.toLowerCase();
	return offered.layouts.find((l) => l.id === settings.id) ?? offered.layouts.find((l) => l.name.toLowerCase() === name);
}

/**
 * Gives a screen a layout from the property inspector, as picking it on the Zones page does.
 * Lit while the screen has that layout. The title is the layout's name.
 */
@action({ UUID: "co.f3nn3x.seam.apply-layout" })
export class ApplyLayout extends TitledAction<Settings> {
	constructor() {
		super(["Apply layout"]);
	}

	protected async render(key: KeyAction<Settings>, settings: Settings, state: SeamState): Promise<void> {
		if (!settings.id && !settings.name) {
			await this.titleKey(key, undefined);
			await key.setState(0);
			return;
		}
		// The layouts give the current name, so a rename in the library shows. A screen that isn't connected has none.
		const number = screenNumber(settings);
		const offered = await seam.layouts(number).catch(() => undefined);
		const layout = offered && chosen(settings, offered);
		await this.titleKey(key, layout?.name ?? settings.name);
		if (number !== undefined) {
			// The state carries each screen's layout, so a fixed screen's lit state needs no call.
			const has = state.screens?.find((s) => s.number === number)?.layout;
			await key.setState(settings.id && has === settings.id ? 1 : 0);
			return;
		}
		await key.setState(layout && layout.id === offered.current ? 1 : 0);
	}

	protected async press(_key: KeyAction<Settings>, settings: Settings): Promise<void> {
		if (!settings.id && !settings.name) {
			throw new Error("no layout chosen");
		}
		const number = screenNumber(settings);
		const send = (layout: string) => seam.call("zones.apply", number === undefined ? { layout } : { number, layout });
		try {
			await send(settings.id ?? settings.name ?? "");
		} catch (error) {
			// The id is gone, say after the layout was deleted and made again. Try its name.
			if (!(error instanceof SeamError) || error.code !== REFUSED || !settings.id || !settings.name) {
				throw error;
			}
			await send(settings.name);
		}
	}

	/** The property inspector asks for its layout dropdown, for the screen it has chosen. */
	override async onSendToPlugin(ev: SendToPluginEvent<JsonValue, Settings>): Promise<void> {
		const payload = ev.payload as { event?: string; screen?: string } | null;
		if (payload?.event !== "getLayouts") {
			return;
		}
		const saved = await ev.action.getSettings();
		const number = screenNumber({ screen: payload.screen });
		let items: { label: string; value: string; disabled?: boolean }[];
		try {
			const offered = await seam.layouts(number);
			items = offered.layouts.map((l) => ({ label: l.name, value: l.id }));
			if (saved.id && !offered.layouts.some((l) => l.id === saved.id)) {
				items.push({ label: saved.name ?? saved.id, value: saved.id });
			}
		} catch (error) {
			const gone = error instanceof SeamError && error.code === NOT_FOUND;
			items = [{ label: gone ? "No such screen right now" : "Seam isn't running", value: "", disabled: true }];
			if (saved.id) {
				items.push({ label: saved.name ?? saved.id, value: saved.id });
			}
		}
		const screens = screenLabels(seam.state?.screens ?? []);
		await streamDeck.ui.sendToPropertyInspector({ event: "getLayouts", items, screens });
	}

	/** Remember the name of whatever was chosen, so a key still reads right when its screen is away. */
	override async onDidReceiveSettings(ev: DidReceiveSettingsEvent<Settings>): Promise<void> {
		const { settings } = ev.payload;
		if (settings.id && seam.status === "online") {
			const offered = await seam.layouts(screenNumber(settings)).catch(() => undefined);
			const layout = offered?.layouts.find((l) => l.id === settings.id);
			if (layout && layout.name !== settings.name) {
				await ev.action.setSettings({ ...settings, name: layout.name });
				return;
			}
		}
		await super.onDidReceiveSettings(ev);
	}
}

type StateScreen = NonNullable<SeamState["screens"]>[number];

/**
 * "1 · DELL U3423WE · 3440×1440 · left, main" for each screen, so the property inspector says
 * which screen is which. The side comes from where the screens sit: left to right when they
 * spread out more across than down, else top to bottom.
 */
function screenLabels(screens: StateScreen[]): { number: number; label: string }[] {
	const placed = screens.filter((s) => s.rect);
	const centre = (s: StateScreen, across: boolean): number =>
		s.rect ? (across ? s.rect.x + s.rect.w / 2 : s.rect.y + s.rect.h / 2) : 0;
	const spread = (across: boolean): number => {
		const at = placed.map((s) => centre(s, across));
		return at.length ? Math.max(...at) - Math.min(...at) : 0;
	};
	const across = spread(true) >= spread(false);
	const order = [...placed].sort((a, b) => centre(a, across) - centre(b, across)).map((s) => s.number);
	const side = (number: number): string | undefined => {
		const at = order.indexOf(number);
		if (order.length < 2 || at < 0) {
			return undefined;
		}
		if (at === 0) {
			return across ? "left" : "top";
		}
		if (at === order.length - 1) {
			return across ? "right" : "bottom";
		}
		return order.length === 3 ? "middle" : `${at + 1} of ${order.length}`;
	};
	return screens.map((s) => {
		const where = [side(s.number), s.primary ? "main" : undefined].filter(Boolean).join(", ");
		const size = s.rect ? `${s.rect.w}×${s.rect.h}` : undefined;
		const parts = [String(s.number), s.name?.trim() || undefined, size, where || undefined].filter(Boolean);
		return { number: s.number, label: parts.join(" · ") };
	});
}
