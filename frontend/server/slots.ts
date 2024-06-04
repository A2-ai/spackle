import type { Slot } from "spackle";
import { info } from "spackle";
import { loadConfig } from "./config";

export async function getSlots(id: string): Promise<Slot[]> {
	const config = await loadConfig();
	if (!config) return [];

	const project = config.projects.find((p) => p.id === id);
	if (!project) return [];

	try {
		return info(project.dir);
	} catch (e) {
		console.error("Error loading slots:", e);
		return [];
	}
}
