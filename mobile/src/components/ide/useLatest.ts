import { useEffect, useRef } from "react";

/** The newest value, readable from callbacks and shortcuts without making them change identity. */
export function useLatest<T>(value: T): { readonly current: T } {
  const ref = useRef(value);
  useEffect(() => {
    ref.current = value;
  });
  return ref;
}
