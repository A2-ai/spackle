import type { Project } from "#/components/Project";

export type ServerConfig = {
	projects: Project[];
};

export async function loadConfig() {
	if (import.meta.env.DEV) {
		return (await Bun.file("testing/server.json").json()) as ServerConfig;
	}

	return (await Bun.file("server.json").json()) as ServerConfig;
}
