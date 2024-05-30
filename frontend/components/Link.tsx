import { type JSX, createMemo } from "solid-js";
import { usePageContext } from "vike-solid/usePageContext";

export function Link(
	props: {
		href: string;
		children: JSX.Element;
	} & JSX.AnchorHTMLAttributes<HTMLAnchorElement>,
) {
	const pageContext = usePageContext();
	const isActive = createMemo(() =>
		props.href === "/"
			? pageContext.urlPathname === props.href
			: pageContext.urlPathname.startsWith(props.href),
	);

	const styles = {
		"py-2 px-4 rounded-2xl disabled:opacity-50 transition ease-in-out bg-sky-600 text-white hover:bg-sky-700": true,
	};

	return (
		<a
			{...props}
			classList={{ "is-active": isActive(), ...styles, ...props.classList }}
		>
			{props.children}
		</a>
	);
}
