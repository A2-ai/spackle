import { TbExclamationMark } from "solid-icons/tb";
import { Show } from "solid-js";
import { usePageContext } from "vike-solid/usePageContext";

export default function Page() {
	const { is404 } = usePageContext();
	return (
		<div class="space-y-4 text-center">
			<TbExclamationMark class="inline text-red-500 text-5xl" />

			<Show
				when={is404}
				fallback={
					<>
						<h1 class="text-3xl text-red-500">500 Internal Server Error</h1>
						<p>Something went wrong.</p>
					</>
				}
			>
				<h1 class="text-3xl text-red-500">404 Page Not Found</h1>
				<p>This page could not be found.</p>
			</Show>
		</div>
	);
}
