import { type SlotType, info } from "../../pkg/spackle";
import { loadConfig } from "./config";

export type Slot = {
	key: string;
	type: SlotType;
	required?: boolean;
	name?: string;
	description?: string;
};

export async function getSlots(id: string): Promise<Slot[]> {
	const config = await loadConfig();
	if (!config) return [];

	const project = config.projects.find((p) => p.id === id);
	if (!project) return [];

	try {
		return info(id).map(
			(s) =>
				({
					key: s.key,
					type: s.type,
					// required: s.required,
					name: s.name,
					description: s.description,
				}) as Slot,
		);
	} catch (e) {
		console.error(e);
		return [];
	}
}
