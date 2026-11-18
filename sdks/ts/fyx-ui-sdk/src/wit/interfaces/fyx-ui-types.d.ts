declare module 'fyx:ui/types@0.1.0' {
  /**
   * Chosen by the extension, unique within one surface. The host keys event routing and
   * patches on it, so an id must not be reused for a different node while the surface lives.
   */
  export type NodeId = number;
  /**
   * Colours are semantic theme slots, never raw values, so every extension follows the
   * active theme (the Zellij approach).
   * # Variants
   * 
   * ## `"text"`
   * 
   * ## `"text-muted"`
   * 
   * ## `"text-accent"`
   * 
   * ## `"error"`
   * 
   * ## `"warning"`
   * 
   * ## `"success"`
   * 
   * ## `"border"`
   * 
   * ## `"surface"`
   * 
   * ## `"elevated-surface"`
   */
  export type ColorToken = "text" | "text-muted" | "text-accent" | "error" | "warning" | "success" | "border" | "surface" | "elevated-surface";
  /**
   * # Variants
   * 
   * ## `"row"`
   * 
   * ## `"column"`
   */
  export type FlexDirection = "row" | "column";
  /**
   * # Variants
   * 
   * ## `"start"`
   * 
   * ## `"center"`
   * 
   * ## `"end"`
   * 
   * ## `"stretch"`
   */
  export type Align = "start" | "center" | "end" | "stretch";
  export interface FlexProps {
    direction: FlexDirection,
    gap: number,
    padding: number,
    grow: boolean,
    align: Align,
    background?: ColorToken,
  }
  export interface TextProps {
    content: string,
    color?: ColorToken,
    bold: boolean,
    monospace: boolean,
  }
  /**
   * # Variants
   * 
   * ## `"default"`
   * 
   * ## `"filled"`
   * 
   * ## `"subtle"`
   * 
   * ## `"danger"`
   */
  export type ButtonStyle = "default" | "filled" | "subtle" | "danger";
  export interface ButtonProps {
    label: string,
    icon?: string,
    style: ButtonStyle,
    disabled: boolean,
  }
  export interface InputProps {
    value: string,
    placeholder: string,
  }
  /**
   * Virtualized: the host asks for rows with `list-rows` as they scroll into view,
   * mirroring GPUI's `uniform_list`. `row-height` is in pixels and fixed for every row.
   */
  export interface ListProps {
    rowCount: number,
    rowHeight: number,
  }
  export interface TableProps {
    columns: Array<string>,
    rowCount: number,
  }
  export interface IconProps {
    name: string,
    color?: ColorToken,
  }
  /**
   * Host component: rendered by Zed's own diff/editor machinery, the extension only supplies text.
   */
  export interface DiffViewProps {
    oldText: string,
    newText: string,
    language?: string,
  }
  export type Props = PropsFlex | PropsText | PropsButton | PropsInput | PropsList | PropsTable | PropsIcon | PropsMarkdown | PropsDiffView;
  export interface PropsFlex {
    tag: 'flex',
    val: FlexProps,
  }
  export interface PropsText {
    tag: 'text',
    val: TextProps,
  }
  export interface PropsButton {
    tag: 'button',
    val: ButtonProps,
  }
  export interface PropsInput {
    tag: 'input',
    val: InputProps,
  }
  export interface PropsList {
    tag: 'list',
    val: ListProps,
  }
  export interface PropsTable {
    tag: 'table',
    val: TableProps,
  }
  export interface PropsIcon {
    tag: 'icon',
    val: IconProps,
  }
  export interface PropsMarkdown {
    tag: 'markdown',
    val: string,
  }
  export interface PropsDiffView {
    tag: 'diff-view',
    val: DiffViewProps,
  }
  /**
   * WIT types cannot be recursive, so a tree travels as a flat `list<node>` in which every
   * node names its parent. Parents precede children; siblings keep list order.
   */
  export interface Node {
    id: NodeId,
    parent?: NodeId,
    props: Props,
  }
  export interface InsertOp {
    node: Node,
    /**
     * Insert before this sibling; `none` appends.
     */
    before?: NodeId,
  }
  export type Patch = PatchSetProps | PatchInsert | PatchRemove;
  export interface PatchSetProps {
    tag: 'set-props',
    val: [NodeId, Props],
  }
  export interface PatchInsert {
    tag: 'insert',
    val: InsertOp,
  }
  /**
   * Removes the node and its whole subtree.
   */
  export interface PatchRemove {
    tag: 'remove',
    val: NodeId,
  }
  export type Update = UpdateUnchanged | UpdateReplace | UpdatePatches;
  export interface UpdateUnchanged {
    tag: 'unchanged',
  }
  export interface UpdateReplace {
    tag: 'replace',
    val: Array<Node>,
  }
  export interface UpdatePatches {
    tag: 'patches',
    val: Array<Patch>,
  }
  export interface ListRow {
    index: number,
    /**
     * A subtree whose single root has `parent: none`.
     */
    nodes: Array<Node>,
  }
  export type UiEvent = UiEventClick | UiEventInputChanged | UiEventInputSubmitted | UiEventRowClick;
  export interface UiEventClick {
    tag: 'click',
    val: NodeId,
  }
  export interface UiEventInputChanged {
    tag: 'input-changed',
    val: [NodeId, string],
  }
  export interface UiEventInputSubmitted {
    tag: 'input-submitted',
    val: [NodeId, string],
  }
  /**
   * The list node and the row index within it.
   */
  export interface UiEventRowClick {
    tag: 'row-click',
    val: [NodeId, number],
  }
  /**
   * # Variants
   * 
   * ## `"left"`
   * 
   * ## `"right"`
   * 
   * ## `"bottom"`
   */
  export type DockPosition = "left" | "right" | "bottom";
  export interface PanelDescriptor {
    position: DockPosition,
    icon: string,
    defaultSize: number,
  }
  /**
   * # Variants
   * 
   * ## `"left"`
   * 
   * ## `"right"`
   */
  export type StatusBarSide = "left" | "right";
  export interface StatusItemDescriptor {
    side: StatusBarSide,
    /**
     * Higher sorts closer to the edge.
     */
    priority: number,
  }
  export type SurfaceKind = SurfaceKindPanel | SurfaceKindStatusItem;
  export interface SurfaceKindPanel {
    tag: 'panel',
    val: PanelDescriptor,
  }
  export interface SurfaceKindStatusItem {
    tag: 'status-item',
    val: StatusItemDescriptor,
  }
  export interface Surface {
    id: string,
    title: string,
    kind: SurfaceKind,
  }
}
