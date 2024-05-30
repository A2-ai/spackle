import type { Config } from "tailwindcss";

export default {
	content: ["./{pages,layouts,components,src}/**/*.{html,js,jsx,ts,tsx,vue}"],
	theme: {
		fontFamily: {
			sans: ["Outfit Variable", "sans-serif"],
		},
		extend: {
			colors: {
				ivory: {
					"50": "#fffff0",
					"100": "#fefec7",
					"200": "#fdfd8a",
					"300": "#fcf64d",
					"400": "#fbea24",
					"500": "#f5cd0b",
					"600": "#d9a106",
					"700": "#b47509",
					"800": "#925a0e",
					"900": "#784a0f",
					"950": "#452703",
				},
			},
		},
	},
} satisfies Config;
