import type { PageContextClient } from "vike/types";

export { data };
export type Data = Awaited<ReturnType<typeof data>>;

async function data(pageContext: PageContextClient) {
	const { id } = pageContext.routeParams || {};

	// const config = await loadConfig();
	// if (!config) return;

	// const project = config.projects.find((p) => p.id === id);
	// if (!project) return;

	// return {
	// 	project,
	// 	slots: await getSlots(project.id),
	// };
}
