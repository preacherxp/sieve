import { handlers } from "./mocks/handlers";

(globalThis as { handlers?: string[] }).handlers = handlers;
