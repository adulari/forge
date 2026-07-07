// T1.4 gallery contribution: Markdown sample, CodeBlock per ported language, StreamingText
// mid-stream + finalized. T1.3's src/app/gallery.tsx registers this section alongside the
// other Batch 1 tasks' `ds/gallery/<task>.tsx` files.
import React from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import { useTheme } from "../../../theme/ThemeProvider";
import { type } from "../../../theme/typography";
import { CodeBlock } from "../../chat/CodeBlock";
import { Markdown } from "../../chat/Markdown";
import { StreamingText } from "../../chat/StreamingText";

const MARKDOWN_SAMPLE = `### Heading

A paragraph with **bold**, *italic*, and \`inline code\`, plus a [link](https://forge.dev).

- first item
- second item with \`code\`
1. step one
2. step two
`;

const CODE_SAMPLES: { language: string; code: string }[] = [
  {
    language: "rust",
    code: 'fn main() {\n    // entry point\n    let count: u32 = 42;\n    println!("hello {}", count);\n}\n',
  },
  {
    language: "js",
    code: "async function greet(name) {\n  // wave\n  const msg = `hi ${name}`;\n  return msg;\n}\n",
  },
  {
    language: "python",
    code: 'def greet(name):\n    # wave\n    count = 42\n    return f"hi {name}"\n',
  },
  {
    language: "go",
    code: 'package main\n\nfunc main() {\n\t// wave\n\tcount := 42\n\tprintln("hi", count)\n}\n',
  },
  {
    language: "bash",
    code: '#!/usr/bin/env bash\n# wave\ncount=42\necho "hi $count"\n',
  },
  {
    language: "json",
    code: '{\n  "name": "forge",\n  "count": 42,\n  "active": true\n}\n',
  },
];

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  const { tokens } = useTheme();
  return (
    <View style={styles.section}>
      <Text style={[type.section, { color: tokens.ink3 }]}>{title}</Text>
      <View style={styles.sectionBody}>{children}</View>
    </View>
  );
}

export default function ContentGallery() {
  const { tokens } = useTheme();

  return (
    <ScrollView style={{ backgroundColor: tokens.bg1 }} contentContainerStyle={styles.container}>
      <Section title="Markdown">
        <Markdown content={MARKDOWN_SAMPLE} />
      </Section>

      <Section title="CodeBlock">
        {CODE_SAMPLES.map((sample) => (
          <View key={sample.language} style={styles.codeSample}>
            <CodeBlock code={sample.code} language={sample.language} />
          </View>
        ))}
      </Section>

      <Section title="StreamingText — mid-stream">
        <StreamingText text="Streaming reply in progress" streaming />
      </Section>

      <Section title="StreamingText — finalized">
        <StreamingText text="This response has finished streaming." streaming={false} />
      </Section>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  container: {
    padding: 16,
    gap: 24,
  },
  section: {
    gap: 8,
  },
  sectionBody: {
    gap: 12,
  },
  codeSample: {
    marginBottom: 4,
  },
});
