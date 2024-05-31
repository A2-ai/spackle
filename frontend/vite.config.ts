import vikeSolid from "vike-solid/vite";
import vike from "vike/plugin";
import { defineConfig } from "vite";
import wasm from "vite-plugin-wasm";

export default defineConfig({
	plugins: [
		vike({
			prerender: {
				partial: true,
			},
		}),
		vikeSolid(),
		wasm(),
	],
	resolve: {
		alias: {
			"#": __dirname,
		},
	},
});
