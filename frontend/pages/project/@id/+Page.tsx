import { TbArrowLeft } from "solid-icons/tb";
import { For } from "solid-js";
import type { Project } from "#/components/ProjectCard";
import SlotField, { type Slot, SlotType } from "#/components/SlotField";

const dummySlots: Slot[] = [
	{
		key: "slot1",
		type: SlotType.String,
		name: "Slot 1",
		description: "Duis ex minim ad id esse.",
		required: true,
	},
	{
		key: "slot2",
		type: SlotType.Number,
		name: "Slot 2",
		description: "Dolore amet deserunt mollit incididunt elit.",
	},
	{
		key: "slot3",
		type: SlotType.Boolean,
		name: "Slot 3",
		description:
			"In in ea cillum laboris Lorem ut nisi esse consectetur commodo anim cupidatat eiusmod reprehenderit.",
	},
];

export default function Page() {
	const project: Project = {};
	return (
		<div class="space-y-4">
			<div class="flex justify-between items-center">
				<h2 class="text-3xl text-gray-800 font-serif">{project?.name}</h2>

				<a href="/" class="text-stone-400">
					<TbArrowLeft class="inline" /> All projects
				</a>
			</div>

			<p class="text-gray-600">{project?.description}</p>

			<form class="my-8 space-y-5">
				<For each={slots}>{(s) => <SlotField slot={s} />}</For>

				<label class="block space-y-1">
					<span class="text-gray-600">Output path</span>
					<input
						type="text"
						class="w-full p-3 rounded-xl bg-stone-50"
						placeholder={`~/projects/${project?.id}`}
					/>
				</label>

				<button
					type="submit"
					class="w-full p-3 rounded-xl bg-orange-50 text-orange-500 hover:bg-orange-400 hover:text-white transition"
				>
					Generate
				</button>
			</form>
		</div>
	);
}
