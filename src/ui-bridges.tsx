import * as React from "react";
import { flushSync } from "react-dom";
import { createRoot, type Root } from "react-dom/client";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";

type InspectorTask = {
  id: string;
  text: string;
  completed: boolean;
};

export type OdoUiBridgeOptions = {
  commandSlot: HTMLElement | null;
  inspectorSlot: HTMLElement | null;
  searchValue: string;
  viewLabel: string;
  shortcutLabel: string;
  canArchive: boolean;
  focusMode: boolean;
  tasks: InspectorTask[];
  inspectorTab: "tasks" | "details";
  noteDetails: {
    folder: string;
    updated: string;
    words: number;
    linkedProject: string;
  } | null;
  onSearch(value: string): void;
  onNewNote(): void;
  onNewFolder(): void;
  onNewTask(): void;
  onGoInbox(): void;
  onToggleSidebar(): void;
  onToggleFocus(): void;
  onArchive(): void;
  onToggleTask(id: string): void;
  onOpenTasks(): void;
  onInspectorTab(value: "tasks" | "details"): void;
};

const roots = new Map<HTMLElement, Root>();

function PhIcon({ name, className = "" }: { name: string; className?: string }) {
  return <i className={`ph ph-${name} ${className}`} aria-hidden="true" />;
}

function mount(slot: HTMLElement | null, node: React.ReactNode) {
  if (!slot) return;
  const root = createRoot(slot);
  roots.set(slot, root);
  flushSync(() => root.render(node));
}

export function unmountOdoUiBridges() {
  roots.forEach((root) => root.unmount());
  roots.clear();
}

function CommandBar(props: OdoUiBridgeOptions) {
  return (
    <TooltipProvider delayDuration={800}>
      <div className="flex h-full min-w-0 items-center gap-3 border-b border-stone-200 bg-stone-50 px-4 text-stone-950">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="outline" size="icon" onClick={props.onGoInbox} aria-label="Back to Inbox" className="border-stone-200 bg-white shadow-none hover:bg-stone-100">
              <PhIcon name="caret-left" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Back to Inbox</TooltipContent>
        </Tooltip>

        <span className="hidden min-w-24 text-sm font-medium text-stone-700 lg:block">{props.viewLabel}</span>

        <label className="relative mx-auto flex h-10 w-full max-w-xl items-center">
          <PhIcon name="magnifying-glass" className="pointer-events-none absolute left-3 size-4 text-stone-500" />
          <Input
            id="search-input"
            type="search"
            defaultValue={props.searchValue}
            onChange={(event) => props.onSearch(event.currentTarget.value)}
            placeholder="Search notes and commands…"
            className="h-10 border-stone-200 bg-stone-100 pl-9 pr-14 text-sm shadow-none placeholder:text-stone-500 focus-visible:border-amber-500 focus-visible:ring-amber-500/20"
          />
          <kbd className="pointer-events-none absolute right-2.5 rounded border border-stone-200 bg-white px-1.5 py-0.5 text-[10px] text-stone-500">{props.shortcutLabel}</kbd>
        </label>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button className="h-10 min-w-28 bg-amber-500 px-4 text-stone-950 shadow-none hover:bg-amber-600" aria-label="Create new">
              <PhIcon name="plus" />
              New
              <PhIcon name="caret-down" className="ml-1 size-3.5" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-48 border-stone-200 bg-white text-stone-950">
            <DropdownMenuItem onSelect={props.onNewNote}><PhIcon name="note-pencil" />New note</DropdownMenuItem>
            <DropdownMenuItem onSelect={props.onNewFolder}><PhIcon name="folder-plus" />New folder</DropdownMenuItem>
            <DropdownMenuItem onSelect={props.onNewTask}><PhIcon name="check-square" />New task</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>

        <Separator orientation="vertical" className="hidden h-6 bg-stone-200 xl:block" />

        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon" onClick={props.onToggleSidebar} aria-label="Toggle sidebar" className="hidden text-stone-700 hover:bg-stone-100 xl:inline-flex">
              <PhIcon name="sidebar" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Toggle sidebar</TooltipContent>
        </Tooltip>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="icon" aria-label="Workspace actions" className="text-stone-700 hover:bg-stone-100">
              <PhIcon name="dots-three-vertical" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-48 border-stone-200 bg-white text-stone-950">
            <DropdownMenuItem onSelect={props.onToggleFocus}><PhIcon name="focus" />{props.focusMode ? "Exit focus" : "Focus mode"}</DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem disabled={!props.canArchive} onSelect={props.onArchive}><PhIcon name="archive" />Archive note</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </TooltipProvider>
  );
}

function Inspector(props: OdoUiBridgeOptions) {
  const openTasks = props.tasks.filter((task) => !task.completed);
  return (
    <TooltipProvider delayDuration={800}>
      <Tabs defaultValue={props.inspectorTab} onValueChange={(value) => props.onInspectorTab(value as "tasks" | "details")} className="h-full gap-0 bg-white">
        <TabsList variant="line" className="h-[52px] w-full justify-start gap-5 border-b border-stone-200 px-5 py-0">
          <TabsTrigger value="tasks" className="h-full flex-none px-0 text-sm data-[state=active]:after:bg-amber-500">Tasks</TabsTrigger>
          <TabsTrigger value="details" className="h-full flex-none px-0 text-sm data-[state=active]:after:bg-amber-500">Details</TabsTrigger>
        </TabsList>

        <TabsContent value="tasks" className="min-h-0">
          <ScrollArea className="h-[calc(100vh-117px)]">
            <div className="p-5">
              <div className="mb-3 flex items-center justify-between">
                <span className="text-xs font-medium text-stone-500">Open work</span>
                <span className="text-xs tabular-nums text-stone-500">{openTasks.length}</span>
              </div>

              <div className="divide-y divide-stone-200 border-y border-stone-200">
                {openTasks.length ? openTasks.map((task) => (
                  <div key={task.id} className="flex min-h-12 items-center gap-3 py-3 text-sm text-stone-700">
                    <Checkbox
                      checked={task.completed}
                      onCheckedChange={() => props.onToggleTask(task.id)}
                      aria-label={`Mark ${task.text} complete`}
                      className="border-stone-300 data-[state=checked]:border-emerald-600 data-[state=checked]:bg-emerald-600"
                    />
                    <span className="min-w-0 flex-1 leading-5">{task.text}</span>
                    <Button variant="ghost" size="icon-xs" aria-label={`More actions for ${task.text}`} className="text-stone-400 hover:bg-stone-100">
                      <PhIcon name="dots-three-vertical" />
                    </Button>
                  </div>
                )) : (
                  <div className="py-8 text-center">
                    <PhIcon name="check-square" className="mx-auto mb-2 size-5 text-emerald-600" />
                    <p className="text-sm text-stone-700">No open tasks</p>
                    <Button variant="link" size="sm" onClick={props.onOpenTasks} className="mt-1 text-amber-700">Open task planner</Button>
                  </div>
                )}
              </div>

              <div className="mt-7">
                <span className="text-xs font-medium text-stone-500">Linked project</span>
                <button type="button" className="mt-3 flex h-11 w-full items-center gap-3 rounded-md border border-stone-200 bg-stone-50 px-3 text-left text-sm text-stone-700 transition-colors hover:bg-stone-100">
                  <PhIcon name="folder" className="size-4 text-stone-500" />
                  <span className="min-w-0 flex-1 truncate">{props.noteDetails?.linkedProject ?? "Summer launch"}</span>
                </button>
              </div>
            </div>
          </ScrollArea>
        </TabsContent>

        <TabsContent value="details" className="p-5">
          {props.noteDetails ? (
            <dl className="grid gap-4 text-sm">
              <div><dt className="text-xs font-medium text-stone-500">Folder</dt><dd className="mt-1 text-stone-700">{props.noteDetails.folder}</dd></div>
              <Separator className="bg-stone-200" />
              <div><dt className="text-xs font-medium text-stone-500">Last updated</dt><dd className="mt-1 text-stone-700">{props.noteDetails.updated}</dd></div>
              <Separator className="bg-stone-200" />
              <div><dt className="text-xs font-medium text-stone-500">Word count</dt><dd className="mt-1 text-stone-700">{props.noteDetails.words}</dd></div>
            </dl>
          ) : <p className="text-sm text-stone-500">Select a note to see its details.</p>}
        </TabsContent>
      </Tabs>
    </TooltipProvider>
  );
}

export function mountOdoUiBridges(options: OdoUiBridgeOptions) {
  mount(options.commandSlot, <CommandBar {...options} />);
  mount(options.inspectorSlot, <Inspector {...options} />);
}
