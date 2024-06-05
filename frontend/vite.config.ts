import native from "rollup-plugin-natives";
import vikeSolid from "vike-solid/vite";
import vike from "vike/plugin";
import { defineConfig } from "vite";
// import native from "vite-plugin-native";

export default defineConfig({
	plugins: [
		vike(),
		vikeSolid(),
		// native({
		// 	target: "esm",
		// }),
		native({
			copyTo: "dist-native",
			targetEsm: true,
			originTransform: (path: string, exists: boolean) => {
				console.error(path, exists);
				return path;
			},
		}),
	],
	resolve: {
		alias: {
			"#": __dirname,
		},
	},
});
