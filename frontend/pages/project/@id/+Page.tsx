import { TbArrowLeft } from "solid-icons/tb";
import { For } from "solid-js";
import tippy from "tippy.js";
import type { Project } from "#/components/Project";
import Slot, { SlotType } from "#/components/Slot";
import type { Slot as SlotT } from "#/components/Slot";
import "tippy.js/dist/tippy.css";

const dummySlots: SlotT[] = [
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
	const project: Project = {
		id: "1",
		name: "Project 1",
		description: "Description of project 1",
	};

	// TODO fetch

	return (
		<div class="space-y-4">
			<div class="flex justify-between items-center">
				<h2 class="text-3xl text-gray-800 ">{project.name}</h2>

				<a href="/" class="text-stone-400">
					<TbArrowLeft class="inline" /> Back
				</a>
			</div>

			<p class="text-gray-600">{project.description}</p>

			<form class="my-8 space-y-5">
				<For each={dummySlots}>{(s) => <Slot slot={s} />}</For>

				<label class="block space-y-1">
					<span class="text-gray-600">Output path</span>
					<input
						type="text"
						class="w-full p-3 rounded-xl bg-stone-50"
						placeholder={`~/projects/${project.id}`}
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
