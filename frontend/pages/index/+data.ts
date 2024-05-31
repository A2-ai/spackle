import type { PageContextServer } from "vike/types";
import type { Project } from "#/components/Project";
import { loadConfig } from "#/server/config";

export { data };
export type Data = Awaited<ReturnType<typeof data>>;

async function data(pageContext: PageContextServer): Promise<Project[]> {
	return (await loadConfig())?.projects || [];
}
