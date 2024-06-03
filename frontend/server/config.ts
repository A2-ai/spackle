import type { Project } from "#/components/Project";

export type ServerConfig = {
	projects: Project[];
};

const CONFIG_FILE = import.meta.env.DEV ? "testing/server.json" : "server.json";

export async function loadConfig(): Promise<ServerConfig | undefined> {
	try {
		return await Bun.file(CONFIG_FILE).json();
	} catch (e) {
		console.error("Error loading config:", e);
		return undefined;
	}
}
