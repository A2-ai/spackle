import type { PageContextServer } from "vike/types";
import { loadConfig } from "#/server/config";
import { getSlots } from "#/server/slots.server";

export { data };
export type Data = Awaited<ReturnType<typeof data>>;

async function data(pageContext: PageContextServer) {
	const { id } = pageContext.routeParams || {};

	const config = loadConfig();
	if (!config) return;

	const project = config.projects.find((p) => p.id === id);
	if (!project) return;

	return {
		project,
		slots: await getSlots(project.id),
	};
}
