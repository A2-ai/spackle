import { normalizeProps, useMachine } from "@zag-js/solid";
import * as zagSwitch from "@zag-js/switch";
import * as tooltip from "@zag-js/tooltip";
import { TbAsterisk } from "solid-icons/tb";
import { Match, Show, Switch, createMemo, createUniqueId } from "solid-js";

export type Slot = {
	key: string;
	type: SlotType;
	required?: boolean;
	name?: string;
	description?: string;
};

export enum SlotType {
	String = "string",
	Number = "number",
	Boolean = "boolean",
}

export default function Slot(props: {
	slot: Slot;
}) {
	const [state, send] = useMachine(
		tooltip.machine({
			id: createUniqueId(),
			openDelay: 0,
			closeDelay: 0,
		}),
	);
	const api = createMemo(() => tooltip.connect(state, send, normalizeProps));

	return (
		<div class="p-5 rounded-2xl bg-stone-50 shadow space-y-3">
			<div class="flex justify-between items-center">
				<h3 class="text-gray-800 inline">{props.slot.name}</h3>
				{props.slot.required && (
					<>
						<button type="button" class="text-rose-400" {...api().triggerProps}>
							<TbAsterisk class="inline" />
						</button>
						<Show when={api().open}>
							<div {...api().positionerProps}>
								<div
									style={{
										"--arrow-size": "10px",
									}}
									{...api().arrowProps}
								>
									<div
										// TODO find a better way to select the right color
										style={{
											"--arrow-background": "rgb(255 241 242)",
										}}
										{...api().arrowTipProps}
									/>
								</div>
								<div
									class="py-2 px-3 rounded-xl text-rose-400 bg-rose-50 shadow-lg"
									{...api().contentProps}
								>
									required
								</div>
							</div>
						</Show>
					</>
				)}
			</div>
			<p class="text-gray-400">{props.slot.description}</p>

			<Switch fallback={<StringSlot slot={props.slot} />}>
				<Match when={props.slot.type === SlotType.String}>
					<StringSlot slot={props.slot} />
				</Match>
				<Match when={props.slot.type === SlotType.Number}>
					<NumberSlot slot={props.slot} />
				</Match>
				<Match when={props.slot.type === SlotType.Boolean}>
					<BooleanSlot slot={props.slot} />
				</Match>
			</Switch>
		</div>
	);
}

export function StringSlot(props: {
	slot: Slot;
}) {
	return <input type="text" class="p-3 rounded-xl w-full" />;
}

export function NumberSlot(props: {
	slot: Slot;
}) {
	return <input type="number" />;
}

export function BooleanSlot(props: {
	slot: Slot;
}) {
	const [state, send] = useMachine(
		zagSwitch.machine({ id: "1", name: props.slot.key }),
	);

	const api = createMemo(() => zagSwitch.connect(state, send, normalizeProps));

	return (
		<label {...api().rootProps}>
			<input {...api().hiddenInputProps} />
			<span {...api().controlProps}>
				<span {...api().thumbProps} />
			</span>
			<span {...api().labelProps}>{api().checked ? "On" : "Off"}</span>
		</label>
	);
}
