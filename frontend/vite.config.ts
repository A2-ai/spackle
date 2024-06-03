import vikeSolid from "vike-solid/vite";
import vike from "vike/plugin";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [
		vike({
			prerender: {
				partial: true,
			},
		}),
		vikeSolid(),
	],
	resolve: {
		alias: {
			"#": __dirname,
		},
	},
	// build: {
	// 	rollupOptions: {
	// 		external: ["spackle"],
	// 	},
	// },
});
