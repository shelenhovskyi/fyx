/// <reference path="./interfaces/fyx-ui-host.d.ts" />
/// <reference path="./interfaces/fyx-ui-types.d.ts" />
declare module 'fyx:ui/ui-extension@0.1.0' {
  export type Surface = import('fyx:ui/types@0.1.0').Surface;
  export type Node = import('fyx:ui/types@0.1.0').Node;
  export type NodeId = import('fyx:ui/types@0.1.0').NodeId;
  export type UiEvent = import('fyx:ui/types@0.1.0').UiEvent;
  export type Update = import('fyx:ui/types@0.1.0').Update;
  export type ListRow = import('fyx:ui/types@0.1.0').ListRow;
  export type * as FyxUiHost010 from 'fyx:ui/host@0.1.0'; // import fyx:ui/host@0.1.0
  export type * as FyxUiTypes010 from 'fyx:ui/types@0.1.0'; // import fyx:ui/types@0.1.0
  /**
  * Called once after instantiation; declares every surface the extension contributes.
  */
  export function surfaces(): Array<Surface>;
  /**
  * Initial tree when the surface is first shown. Not called per frame: the host retains the tree.
  *
  * @throws {string} To return the error of the result, throw it, or an `Error` that has it as its `payload`.
  */
  export function mount(surfaceId: string): Array<Node>;
  /**
  * At most one call per input event; the host debounces scroll, hover and typing.
  *
  * @throws {string} To return the error of the result, throw it, or an `Error` that has it as its `payload`.
  */
  export function handleEvent(surfaceId: string, event: UiEvent): Update;
  export function listRows(surfaceId: string, listNode: NodeId, start: number, count: number): Array<ListRow>;
  export type Result<T, E> = { tag: 'ok', val: T } | { tag: 'err', val: E };
}
