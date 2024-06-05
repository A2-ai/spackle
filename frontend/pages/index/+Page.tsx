import { TbX } from "solid-icons/tb";
import { For, Show, createSignal } from "solid-js";
import { useData } from "vike-solid/useData";
import Project, { dummyProjects } from "#/components/Project";
import type { Data } from "./+data";

export default function Page() {
	const [search, setSearch] = createSignal("");
	const projects = useData<Data>();

	const filteredProjects = () => {
		return projects.filter((p) => {
			const nameMatch = p.name.toLowerCase().includes(search().toLowerCase());

			const descriptionMatch = p.description
				?.toLowerCase()
				.includes(search().toLowerCase());

			return nameMatch || descriptionMatch;
		});
	};

	return (
		<div class="space-y-6">
			<div class="relative">
				<input
					type="text"
					placeholder="Search for projects and their descriptions..."
					class="w-full p-3 rounded-xl bg-stone-50"
					oninput={(e) => {
						setSearch(e.currentTarget.value);
					}}
					value={search()}
				/>
				<Show when={search().length > 0}>
					<button
						class="text-stone-300 absolute top-1/2 right-3 -translate-y-1/2"
						type="button"
						onClick={() => {
							setSearch("");
						}}
					>
						<TbX />
					</button>
				</Show>
			</div>

			<div class="space-y-4">
				<For each={filteredProjects()}>{(p) => <Project project={p} />}</For>

				{filteredProjects().length === 0 && (
					<p class="text-center text-slate-500 p-6">
						No projects found
						{search().length > 0 ? ` matching ${search()}` : ""}
					</p>
				)}
			</div>
		</div>
	);
}
