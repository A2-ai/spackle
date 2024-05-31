import { TbTrowel } from "solid-icons/tb";
import type { JSX } from "solid-js";

import "./tailwind.css";
import "@fontsource-variable/outfit";
import "@fontsource/alice";

export default function LayoutDefault(props: { children?: JSX.Element }) {
	return (
		<div class="max-w-screen-lg m-auto p-2 lg:p-8">
			<Logo />

			<div class="p-5 pb-12 min-h-screen">{props.children}</div>
		</div>
	);
}

function Logo() {
	return (
		<a href="/" class="text-center">
			<h1 class="text-3xl font-medium font-serif text-orange-600">
				<TbTrowel class="inline mr-1" />
				Spackle
			</h1>
		</a>
	);
}
