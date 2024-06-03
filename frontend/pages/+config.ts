import vikeSolid from "vike-solid/config";
import type { Config } from "vike/types";
import Head from "../layouts/HeadDefault.jsx";
import Layout from "../layouts/LayoutDefault.jsx";

export default {
	Layout,
	Head,
	title: "Spackle",
	extends: vikeSolid,
	ssr: true,
} satisfies Config;
