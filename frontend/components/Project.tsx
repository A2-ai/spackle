import { TbArrowRight } from "solid-icons/tb";

export type Project = {
	id: string;
	name: string;
	description?: string;
	sourceUrl?: string;
};

export const dummyProjects: Project[] = [
	{
		id: "1",
		name: "Project 1",
		description: "Description of project 1",
	},
	{
		id: "2",
		name: "Project 2",
		description: "Description of project 2",
	},
	{
		id: "3",
		name: "Project 3",
		description: "Description of project 3",
	},
];

export default function Project(props: {
	project: Project;
}) {
	return (
		<a
			class="p-6 rounded-2xl bg-stone-100 flex justify-between items-center shadow"
			href={`/project/${props.project.id}`}
		>
			<div>
				<h2 class="text-xl text-stone-700">{props.project.name}</h2>
				<p class="text-stone-500">{props.project.description}</p>
			</div>

			<TbArrowRight class="text-stone-300 text-4xl" />
		</a>
	);
}
