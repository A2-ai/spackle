import vikeSolid from "vike-solid/vite";
import vike from "vike/plugin";
import { defineConfig } from "vite";
import native from "vite-plugin-native";

export default defineConfig({
	plugins: [
		vike(),
		vikeSolid(),
		native({
			target: "esm",
		}),
	],
	resolve: {
		alias: {
			"#": __dirname,
		},
	},
	optimizeDeps: {
		exclude: ["spackle"],
	},
	// build: {
	// 	commonjsOptions: {
	// 		include: [/spackle/, /node_modules/],
	// 	},
	// },
});
