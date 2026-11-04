// Small React hooks shared by the screens.
import { useEffect, useRef, useState } from "react";

/** Calls `f` once the value stopped changing for `ms`: a slider being
    dragged writes the settings once, not at every step. */
export function useDebounced<T>(f: (value: T) => void, ms = 150): (value: T) => void {
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const latest = useRef(f);
  latest.current = f;
  useEffect(() => () => clearTimeout(timer.current), []);
  return (value: T) => {
    clearTimeout(timer.current);
    timer.current = setTimeout(() => latest.current(value), ms);
  };
}

/** A value the person drags, kept apart from the one the Rust side reports
    while the drag lasts, so the poll never pulls the slider back. */
export function useDragged(reported: number, save: (value: number) => void) {
  const [value, setValue] = useState(reported);
  const dragging = useRef(false);
  useEffect(() => {
    if (!dragging.current) setValue(reported);
  }, [reported]);
  const saveLater = useDebounced((v: number) => {
    dragging.current = false;
    save(v);
  });
  const change = (v: number) => {
    dragging.current = true;
    setValue(v);
    saveLater(v);
  };
  return [value, change] as const;
}

/** Below this width the window has no room for the side menu: the spaces
    become the first screen, as on a phone (the canvas's narrow window). */
export const NARROW_PX = 640;

export function useNarrow(): boolean {
  const query = `(max-width: ${NARROW_PX - 1}px)`;
  const [narrow, setNarrow] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const list = window.matchMedia(query);
    const change = () => setNarrow(list.matches);
    list.addEventListener("change", change);
    return () => list.removeEventListener("change", change);
  }, [query]);
  return narrow;
}
