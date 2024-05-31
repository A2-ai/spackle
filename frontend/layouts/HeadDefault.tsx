/* eslint-disable solid/no-innerhtml */

import icon from "../assets/tb-trowel.svg";

export default function HeadDefault() {
	return (
		<>
			<meta name="viewport" content="width=device-width, initial-scale=1" />
			<link rel="icon" href={icon} />
		</>
	);
}
