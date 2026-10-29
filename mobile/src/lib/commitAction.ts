import AsyncStorage from "@react-native-async-storage/async-storage";
import { useCallback, useEffect, useState } from "react";

import { type CommitAction, parseCommitAction } from "../components/git/gitReviewSupport";

const KEY = "forge.git.commitAction";

/** The commit button's remembered primary action. Storage that is unavailable or slow just means
 * the default (plain commit); the choice is a convenience, never state anything depends on. */
export function useCommitAction(): [CommitAction, (next: CommitAction) => void] {
  const [action, setAction] = useState<CommitAction>("commit");
  useEffect(() => {
    let live = true;
    AsyncStorage.getItem(KEY)
      .then((stored) => {
        if (live) setAction(parseCommitAction(stored));
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);
  const choose = useCallback((next: CommitAction) => {
    setAction(next);
    AsyncStorage.setItem(KEY, next).catch(() => undefined);
  }, []);
  return [action, choose];
}
