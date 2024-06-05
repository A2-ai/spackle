import type { Slot } from "spackle";
import { getSlots as getSlotsNative } from "spackle";
import { loadConfig } from "./config";
// import { getSlots as getSlotsNative } from "./addon.server";
// import type { Slot, SlotType } from "./addon.server";

export async function getSlots(id: string): Promise<Slot[]> {
	const config = loadConfig();
	if (!config) return [];

	const project = config.projects.find((p) => p.id === id);
	if (!project) return [];

	try {
		return getSlotsNative(project.dir);
	} catch (e) {
		console.error("Error loading slots:", e);
		return [];
	}
}
