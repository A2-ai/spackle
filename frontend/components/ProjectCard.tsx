import { TbArrowRight } from "solid-icons/tb";

export type Project = {
	id: string;
	dir: string;
	name: string;
	description?: string;
	sourceUrl?: string;
};

export const dummyProjects: Project[] = [
	{
		id: "1",
		name: "Project 1",
		dir: "../tests/data/proj1",
		description: "Description of project 1",
		sourceUrl: "https://github.com",
	},
	{
		id: "2",
		name: "Project 2",
		dir: "../tests/data/proj1",
		description: "Description of project 2",
	},
	{
		id: "3",
		name: "Project 3",
		dir: "../tests/data/proj1",
		description: "Description of project 3",
	},
];

export default function ProjectCard(props: {
	project: Project;
}) {
	return (
		<a
			class="p-6 rounded-2xl bg-stone-100 flex justify-between items-center shadow"
			href={`/project/${props.project.id}`}
		>
			<div>
				<h2 class="text-2xl text-stone-700  font-serif">
					{props.project.name}
				</h2>
				<p class="text-stone-500">{props.project.description}</p>
			</div>

			<TbArrowRight class="text-stone-300 text-4xl" />
		</a>
	);
}
