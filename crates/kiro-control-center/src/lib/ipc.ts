import type { CommandError } from "$lib/bindings";

/** Result shape produced by `typedError<T, CommandError>` in the generated bindings. */
export type IpcResult<T> =
  | { status: "ok"; data: T }
  | { status: "error"; error: CommandError };
