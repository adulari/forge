// BUILD_ORDER T1.3 gallery registry — every ds/ container in every state, both
// themes (the parent gallery route owns the theme toggle), both breakpoints
// visible at once where practical (MasterDetail is left to live window resize).
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { Archive, Flame, Inbox } from "lucide-react-native";

import { useTokens } from "../../../theme/ThemeProvider";
import { space } from "../../../theme/tokens";
import { type } from "../../../theme/typography";
import { Banner } from "../Banner";
import { BoundedList } from "../BoundedList";
import { Card } from "../Card";
import { ConfirmDialog } from "../ConfirmDialog";
import { EmptyState } from "../EmptyState";
import { ListRow } from "../ListRow";
import { MasterDetail } from "../MasterDetail";
import { Sheet } from "../Sheet";
import { Skeleton, SkeletonRow } from "../Skeleton";
import { useToast } from "../ToastHost";

function SectionLabel({ children }: { children: string }) {
  const tokens = useTokens();
  return <Text style={[type.section, styles.sectionLabel, { color: tokens.ink3 }]}>{children}</Text>;
}

const DEMO_ITEMS = ["alpha", "bravo", "charlie", "delta", "echo"];

export default function ContainersGallery() {
  const tokens = useTokens();
  const toast = useToast();
  const [sheetOpen, setSheetOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [destructiveOpen, setDestructiveOpen] = useState(false);
  const [listEmpty, setListEmpty] = useState(false);

  return (
    <View style={styles.container}>
      <SectionLabel>Card</SectionLabel>
      <Card style={styles.gap}>
        <Text style={[type.heading, { color: tokens.ink }]}>Default card</Text>
        <Text style={[type.sub, { color: tokens.ink2 }]}>bg2, radius 12, hairline border.</Text>
      </Card>
      <Card variant="feature" style={styles.gap}>
        <Text style={[type.heading, { color: tokens.ink }]}>Feature card</Text>
        <Text style={[type.sub, { color: tokens.ink2 }]}>radius 16 — plan/diff/permission cards.</Text>
      </Card>

      <SectionLabel>ListRow</SectionLabel>
      <Card padded={false}>
        <ListRow
          title="Pressable row"
          subtitle="with subtitle + Strike"
          onPress={() => toast.show("row pressed")}
          leading={<Flame size={20} color={tokens.accent} strokeWidth={1.75} />}
        />
        <ListRow title="Static row" subtitle="no onPress, no Strike" showSeparator={false} />
      </Card>

      <SectionLabel>BoundedList</SectionLabel>
      <Pressable
        onPress={() => setListEmpty((v) => !v)}
        accessibilityRole="button"
        accessibilityLabel="Toggle BoundedList empty state"
        style={[styles.toggle, { borderColor: tokens.border }]}
      >
        <Text style={[type.meta, { color: tokens.ink2 }]}>{listEmpty ? "show items" : "show empty state"}</Text>
      </Pressable>
      <View style={[styles.listBox, { borderColor: tokens.border }]}>
        <BoundedList
          data={listEmpty ? [] : DEMO_ITEMS}
          keyExtractor={(item) => item}
          renderItem={({ item }) => <ListRow title={item} />}
          ListEmptyComponent={<EmptyState icon={Inbox} message="nothing here yet" />}
        />
      </View>

      <SectionLabel>Sheet (Anvil)</SectionLabel>
      <Pressable
        onPress={() => setSheetOpen(true)}
        accessibilityRole="button"
        accessibilityLabel="Open demo sheet"
        style={[styles.toggle, { borderColor: tokens.border }]}
      >
        <Text style={[type.meta, { color: tokens.ink2 }]}>open sheet</Text>
      </Pressable>
      <Sheet visible={sheetOpen} onClose={() => setSheetOpen(false)} snapPoints={[0.4, 0.9]} accessibilityLabel="Demo sheet">
        <View style={styles.sheetBody}>
          <Text style={[type.heading, { color: tokens.ink }]}>Anvil sheet</Text>
          <Text style={[type.sub, { color: tokens.ink2 }]}>
            Drag the grabber between snap points, or drag past the threshold to dismiss.
          </Text>
        </View>
      </Sheet>

      <SectionLabel>Toast (Signal)</SectionLabel>
      <Pressable
        onPress={() => toast.show("saved", { tone: "success" })}
        accessibilityRole="button"
        accessibilityLabel="Trigger a toast"
        style={[styles.toggle, { borderColor: tokens.border }]}
      >
        <Text style={[type.meta, { color: tokens.ink2 }]}>show toast</Text>
      </Pressable>

      <SectionLabel>Banner</SectionLabel>
      <Banner tone="warn" message="protocol mismatch — update the app to continue." style={styles.gap} />
      <Banner tone="danger" message="this session is publicly exposed." style={styles.gap} />
      <Banner tone="neutral" compact message="reconnecting…" style={styles.gap} />

      <SectionLabel>EmptyState</SectionLabel>
      <Card style={styles.gap}>
        <EmptyState icon={Archive} message="no archived sessions yet." />
      </Card>

      <SectionLabel>Skeleton (Temper)</SectionLabel>
      <Card style={styles.gap}>
        <SkeletonRow />
        <Skeleton width="80%" height={14} style={styles.skeletonGap} />
      </Card>

      <SectionLabel>ConfirmDialog</SectionLabel>
      <View style={styles.row}>
        <Pressable
          onPress={() => setConfirmOpen(true)}
          accessibilityRole="button"
          accessibilityLabel="Open confirm dialog"
          style={[styles.toggle, { borderColor: tokens.border }]}
        >
          <Text style={[type.meta, { color: tokens.ink2 }]}>confirm</Text>
        </Pressable>
        <Pressable
          onPress={() => setDestructiveOpen(true)}
          accessibilityRole="button"
          accessibilityLabel="Open destructive confirm dialog"
          style={[styles.toggle, { borderColor: tokens.danger }]}
        >
          <Text style={[type.meta, { color: tokens.danger }]}>destructive</Text>
        </Pressable>
      </View>
      <ConfirmDialog
        visible={confirmOpen}
        title="Leave without saving?"
        message="Your draft will be lost."
        confirmLabel="Leave"
        onConfirm={() => setConfirmOpen(false)}
        onCancel={() => setConfirmOpen(false)}
      />
      <ConfirmDialog
        visible={destructiveOpen}
        title="Discard branch forge/subagent/ab12?"
        message="Unmerged work is lost."
        confirmLabel="Discard"
        destructive
        onConfirm={() => setDestructiveOpen(false)}
        onCancel={() => setDestructiveOpen(false)}
      />

      <SectionLabel>MasterDetail (resize the window at expanded, ≥1024pt)</SectionLabel>
      <View style={[styles.masterDetailBox, { borderColor: tokens.border }]}>
        <MasterDetail
          master={
            <View style={styles.paneDemo}>
              <Text style={[type.sub, { color: tokens.ink2 }]}>rail (master)</Text>
            </View>
          }
          detail={
            <View style={styles.paneDemo}>
              <Text style={[type.sub, { color: tokens.ink2 }]}>detail pane</Text>
            </View>
          }
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { gap: space.space8, paddingBottom: space.space32 },
  sectionLabel: { marginTop: space.space24 },
  gap: { gap: space.space4 },
  toggle: {
    alignSelf: "flex-start",
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
  },
  row: { flexDirection: "row", gap: space.space8 },
  listBox: { height: 220, borderWidth: StyleSheet.hairlineWidth, borderRadius: 12 },
  sheetBody: { padding: space.space16, gap: space.space8 },
  skeletonGap: { marginTop: space.space8 },
  masterDetailBox: { height: 200, borderWidth: StyleSheet.hairlineWidth, borderRadius: 12, overflow: "hidden" },
  paneDemo: { flex: 1, padding: space.space16 },
});
