import streamDeck from "@elgato/streamdeck";
import net from "node:net";
import os from "node:os";

/** What Seam reports about itself, trimmed to what the keys use. */
export type SeamState = {
	remap: "on" | "paused" | "off_no_borders";
	zones: "on" | "off";
	desk: { sig: string; name: string };
	workspace: { id: string; name: string } | null;
};

export type Workspace = { id: string; name: string; active: boolean };

/** `offline` is no pipe, `blocked` is Seam answering that other apps are switched off. */
export type Status = "online" | "offline" | "blocked";

/** Seam's error codes (api.md), plus one of ours for a call that never left. */
export const NOT_FOUND = 1;
export const DISABLED = 5;
export const OFFLINE = -1;

export class SeamError extends Error {
	constructor(
		readonly code: number,
		message: string,
	) {
		super(message);
	}
}

const MIN_DELAY = 1_000;
const MAX_DELAY = 30_000;
const CALL_TIMEOUT = 15_000;
const TOPICS = ["state", "workspace", "desk"];

type Pending = { resolve: (value: never) => void; reject: (error: SeamError) => void; timer: NodeJS.Timeout };

/** `\\.\pipe\seam-<username>`, derived the way the tray derives it. */
export function pipeName(): string {
	const user = process.env.USERNAME ?? os.userInfo().username;
	return `\\\\.\\pipe\\seam-${user.toLowerCase()}`;
}

/**
 * The one connection every key shares. It reconnects on its own, keeps the last
 * State it was given, and tells listeners whenever the status or the State changes.
 */
class SeamConnection {
	status: Status = "offline";
	state: SeamState | undefined;

	#socket: net.Socket | undefined;
	#buffer = "";
	#nextId = 1;
	#pending = new Map<number, Pending>();
	#delay = MIN_DELAY;
	#refused = false;
	#listeners = new Set<() => void>();

	start(): void {
		this.#connect();
	}

	/** Runs `listener` after every change of status or State. */
	onChange(listener: () => void): void {
		this.#listeners.add(listener);
	}

	async call<T = unknown>(method: string, params: object = {}): Promise<T> {
		if (this.status !== "online") {
			throw new SeamError(OFFLINE, "Seam isn't running");
		}
		return this.#send<T>(method, params);
	}

	workspaces(): Promise<Workspace[]> {
		return this.call<Workspace[]>("workspaces.list");
	}

	#changed(): void {
		for (const listener of this.#listeners) {
			listener();
		}
	}

	#connect(): void {
		const socket = net.connect(pipeName());
		socket.setEncoding("utf8");
		socket.once("connect", () => {
			this.#socket = socket;
			this.#buffer = "";
			void this.#handshake(socket);
		});
		socket.on("data", (chunk: string) => this.#receive(chunk));
		socket.on("error", (error) => streamDeck.logger.debug(`Seam pipe: ${error.message}`));
		socket.once("close", () => this.#lost(socket));
	}

	async #handshake(socket: net.Socket): Promise<void> {
		try {
			await this.#send("hello", { client: "streamdeck", version: streamDeck.info.plugin.version });
			await this.#send("events.subscribe", { topics: TOPICS });
			this.state = await this.#send<SeamState>("state.get");
			this.#delay = MIN_DELAY;
			this.status = "online";
			streamDeck.logger.info("Connected to Seam");
			this.#changed();
		} catch (error) {
			this.#refused = error instanceof SeamError && error.code === DISABLED;
			streamDeck.logger.warn(`Seam refused the connection: ${(error as Error).message}`);
			socket.destroy();
		}
	}

	#lost(socket: net.Socket): void {
		if (socket === this.#socket) {
			this.#socket = undefined;
		}
		for (const [id, call] of this.#pending) {
			clearTimeout(call.timer);
			call.reject(new SeamError(OFFLINE, "Seam isn't running"));
			this.#pending.delete(id);
		}
		const next: Status = this.#refused ? "blocked" : "offline";
		this.#refused = false;
		const changed = this.status !== next || this.state !== undefined;
		this.status = next;
		this.state = undefined;
		if (changed) {
			this.#changed();
		}
		const delay = this.#delay;
		this.#delay = Math.min(delay * 2, MAX_DELAY);
		setTimeout(() => this.#connect(), delay);
	}

	#send<T>(method: string, params: object = {}): Promise<T> {
		const socket = this.#socket;
		if (!socket) {
			return Promise.reject(new SeamError(OFFLINE, "Seam isn't running"));
		}
		const id = this.#nextId++;
		return new Promise<T>((resolve, reject) => {
			const timer = setTimeout(() => {
				this.#pending.delete(id);
				reject(new SeamError(OFFLINE, "Seam didn't answer in time"));
			}, CALL_TIMEOUT);
			this.#pending.set(id, { resolve: resolve as (value: never) => void, reject, timer });
			socket.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
		});
	}

	#receive(chunk: string): void {
		this.#buffer += chunk;
		let end: number;
		while ((end = this.#buffer.indexOf("\n")) >= 0) {
			const line = this.#buffer.slice(0, end).trim();
			this.#buffer = this.#buffer.slice(end + 1);
			if (line) {
				this.#handle(line);
			}
		}
	}

	#handle(line: string): void {
		let message: {
			id?: number;
			result?: never;
			error?: { code: number; message: string };
			method?: string;
			params?: { type: string; data: unknown };
		};
		try {
			message = JSON.parse(line);
		} catch {
			streamDeck.logger.warn("Seam sent a line that isn't JSON");
			return;
		}
		if (message.method === "event" && message.params) {
			this.#event(message.params.type, message.params.data);
			return;
		}
		const call = message.id === undefined ? undefined : this.#pending.get(message.id);
		if (!call || message.id === undefined) {
			return;
		}
		this.#pending.delete(message.id);
		clearTimeout(call.timer);
		if (message.error) {
			call.reject(new SeamError(message.error.code, message.error.message));
		} else {
			call.resolve(message.result as never);
		}
	}

	#event(type: string, data: unknown): void {
		if (this.status !== "online") {
			return;
		}
		if (type === "state.changed") {
			this.state = data as SeamState;
			this.#changed();
		} else if (type === "desk.changed" || type === "workspace.switched") {
			// The State follows, but asking costs nothing and keeps the keys honest.
			this.#send<SeamState>("state.get")
				.then((state) => {
					this.state = state;
					this.#changed();
				})
				.catch((error: Error) => streamDeck.logger.warn(`state.get failed: ${error.message}`));
		}
	}
}

export const seam = new SeamConnection();
