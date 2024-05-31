import type { Config } from "tailwindcss";
import defaultTheme from "tailwindcss/defaultTheme";

export default {
	content: ["./{pages,layouts,components,src}/**/*.{html,js,jsx,ts,tsx,vue}"],
	theme: {
		extend: {
			fontFamily: {
				sans: ["Outfit Variable", ...defaultTheme.fontFamily.sans],
				serif: ["Alice", ...defaultTheme.fontFamily.serif],
			},
		},
	},
} satisfies Config;
