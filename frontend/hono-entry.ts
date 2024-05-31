import { serve } from "@hono/node-server";
import { serveStatic } from "@hono/node-server/serve-static";
import { Hono } from "hono";
import { compress } from "hono/compress";
import { createMiddleware } from "hono/factory";
import { vikeHandler } from "./server/vike-handler";

const isProduction = process.env.NODE_ENV === "production";
const port = process.env.PORT ? Number.parseInt(process.env.PORT, 10) : 3000;

type Middleware<Context extends Record<string | number | symbol, unknown>> = (
	request: Request,
	context: Context,
) => Response | void | Promise<Response> | Promise<void>;

export function handlerAdapter<
	Context extends Record<string | number | symbol, unknown>,
>(handler: Middleware<Context>) {
	return createMiddleware(async (context, next) => {
		let ctx = context.get("context");
		if (!ctx) {
			ctx = {};
			context.set("context", ctx);
		}

		const res = await handler(context.req.raw, ctx as Context);
		context.set("context", ctx);

		if (!res) {
			await next();
		}

		return res;
	});
}

const app = new Hono();

app.use(compress());

if (isProduction) {
	app.use(
		"/*",
		serveStatic({
			root: "dist/client/",
		}),
	);
}

/**
 * Vike route
 *
 * @link {@see https://vike.dev}
 **/
app.all("*", handlerAdapter(vikeHandler));

if (isProduction) {
	console.log(`Server listening on http://localhost:${port}`);
	serve({
		fetch: app.fetch,
		port: port,
	});
}

export default app;
