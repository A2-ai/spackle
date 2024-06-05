import vikeSolid from "vike-solid/config";
import type { Config } from "vike/types";
import Head from "../layouts/HeadDefault.js";
import Layout from "../layouts/LayoutDefault.js";

export default {
	Layout,
	Head,
	title: "Spackle",
	extends: vikeSolid,
	ssr: true,
	// clientRouting: false,
} satisfies Config;
