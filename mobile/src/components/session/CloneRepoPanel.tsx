// "Clone a repository" body for the project picker sheet: a URL field (the daemon's allow-list
// message is shown verbatim when it refuses one) and, when `gh` is signed in on the host, a
// searchable list of the user's repositories that fills the URL.
import { ArrowLeft, Lock } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { ApiError } from "../../lib/api";
import { cloneNameFromUrl, cloneUrlProblem, filterGhRepos } from "../../lib/projectOnboarding";
import { useCloneProject, useGhRepos } from "../../lib/queriesProjects";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { ListRow } from "../ds/ListRow";
import { SearchField } from "../ds/SearchField";
import { SectionHeader } from "../ds/SectionHeader";
import { Skeleton } from "../ds/Skeleton";

const REPO_ROWS = 40;

export function CloneRepoPanel({ onBack, onCloned }: { onBack: () => void; onCloned: (path: string) => void }) {
  const tokens = useTokens();
  const clone = useCloneProject();
  const gh = useGhRepos(true);
  const [url, setUrl] = useState("");
  const [folder, setFolder] = useState("");
  const [parent, setParent] = useState("");
  const [query, setQuery] = useState("");
  const [localProblem, setLocalProblem] = useState<string | null>(null);

  const repos = useMemo(() => filterGhRepos(gh.data?.repos ?? [], query).slice(0, REPO_ROWS), [gh.data, query]);
  const serverError = clone.error instanceof ApiError ? clone.error.message : clone.isError ? "Clone failed" : null;
  const derivedName = cloneNameFromUrl(url);

  const submit = () => {
    const problem = cloneUrlProblem(url);
    setLocalProblem(problem);
    if (problem || clone.isPending) return;
    clone.mutate(
      { url: url.trim(), name: folder.trim() || undefined, parent: parent.trim() || undefined },
      { onSuccess: (result) => onCloned(result.path) },
    );
  };

  return (
    <View style={styles.wrap}>
      <ListRow
        title="Back to projects"
        leading={<ArrowLeft size={16} strokeWidth={1.75} color={tokens.ink2} />}
        onPress={onBack}
        showSeparator={false}
      />
      <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Clone a repository</Text>
      <Input
        label="Repository URL"
        value={url}
        onChangeText={(text) => {
          setUrl(text);
          setLocalProblem(null);
          if (clone.isError) clone.reset();
        }}
        onSubmitEditing={submit}
        placeholder="https://github.com/owner/repo.git"
        mono
        autoCapitalize="none"
        autoCorrect={false}
        keyboardType="url"
        error={localProblem ?? serverError ?? undefined}
        disabled={clone.isPending}
        accessibilityLabel="Repository URL"
      />
      <View style={styles.pair}>
        <Input
          label="Folder name"
          value={folder}
          onChangeText={setFolder}
          placeholder={derivedName || "repo"}
          mono
          autoCapitalize="none"
          autoCorrect={false}
          disabled={clone.isPending}
          containerStyle={styles.pairField}
          accessibilityLabel="Folder name"
        />
        <Input
          label="Clone into"
          value={parent}
          onChangeText={setParent}
          placeholder="~/Projects"
          mono
          autoCapitalize="none"
          autoCorrect={false}
          disabled={clone.isPending}
          containerStyle={styles.pairField}
          accessibilityLabel="Clone into folder"
        />
      </View>
      <Button
        label={clone.isPending ? `Cloning ${derivedName || "repository"}…` : "Clone"}
        onPress={submit}
        loading={clone.isPending}
        disabled={!url.trim()}
        fullWidth
      />
      {clone.isPending ? (
        <Text accessibilityLiveRegion="polite" style={[typeScale.sub, { color: tokens.ink3 }]}>
          Large repositories can take a few minutes. You can keep this sheet open.
        </Text>
      ) : null}

      <View>
        <SectionHeader>Your GitHub repositories</SectionHeader>
        {gh.isLoading ? (
          [0, 1, 2].map((index) => <Skeleton key={index} height={56} width="100%" />)
        ) : gh.data?.available ? (
          <>
            <SearchField value={query} onChangeText={setQuery} placeholder="Search repositories" accessibilityLabel="Search your repositories" />
            {repos.length === 0 ? (
              <Text style={[typeScale.sub, styles.note, { color: tokens.ink3 }]}>No repository matches.</Text>
            ) : (
              repos.map((repo, index) => (
                <ListRow
                  key={repo.url}
                  title={repo.name_with_owner}
                  subtitle={repo.description || undefined}
                  trailing={repo.private ? <Lock size={14} strokeWidth={1.75} color={tokens.ink3} /> : <Badge label="public" tone="outline" lowercase />}
                  onPress={() => {
                    setUrl(repo.url);
                    setLocalProblem(null);
                    clone.reset();
                  }}
                  disabled={clone.isPending}
                  showSeparator={index < repos.length - 1}
                  accessibilityLabel={`Use ${repo.name_with_owner}`}
                />
              ))
            )}
          </>
        ) : (
          <Text style={[typeScale.sub, styles.note, { color: tokens.ink3 }]}>
            {gh.isError ? "Could not load your repositories." : gh.data?.note || "GitHub CLI is not signed in on this host."}
          </Text>
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 },
  pair: { flexDirection: "row", flexWrap: "wrap", gap: space.space12 },
  pairField: { flexGrow: 1, flexBasis: 160 },
  note: { paddingVertical: space.space8 },
});
