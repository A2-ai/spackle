import fs from "node:fs";
import type { Project } from "#/components/Project";

export type ServerConfig = {
	projects: Project[];
};

const CONFIG_FILE = import.meta.env.DEV ? "testing/server.json" : "server.json";

export function loadConfig(): ServerConfig | undefined {
	try {
		return JSON.parse(fs.readFileSync(CONFIG_FILE, "utf8")) as ServerConfig;
	} catch (e) {
		console.error("Error loading config:", e);
		return undefined;
	}
}
