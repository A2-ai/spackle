import type { PageContextServer } from "vike/types";
import type { Project } from "#/components/Project";
import type { ServerConfig } from "#/server/config";

export { data };
export type Data = Awaited<ReturnType<typeof data>>;

async function data(
	pageContext: PageContextServer,
): Promise<Project | undefined> {
	const { id } = pageContext.routeParams || {};

	const config: ServerConfig = await Bun.file("testing/server.json").json();

	return config.projects.find((p) => p.id === id);
}
