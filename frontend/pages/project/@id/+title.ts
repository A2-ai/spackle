import type { PageContext } from "vike/types";
// import type { Data } from "./+data";

export function title(pageContext: PageContext) {
	const { project } = pageContext.data || {};
	return `${project?.name || "???"} - Spackle`;
}
