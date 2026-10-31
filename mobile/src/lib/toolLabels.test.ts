import { describe, expect, it } from "vitest";

import { callHeadline, callVerb, readableArgs } from "./toolLabels";

describe("python kernel tool labels", () => {
  const args = JSON.stringify({ code: "# load\nimport pandas as pd\ndf = pd.read_csv('x.csv')" });

  it("says Python and shows the first code line", () => {
    expect(callVerb("python")).toBe("Python");
    expect(callHeadline("python", args, null)).toBe("import pandas as pd");
  });

  it("expands to the whole cell, not JSON-escaped code", () => {
    expect(readableArgs("python", args)).toEqual([
      { label: "code", text: "# load\nimport pandas as pd\ndf = pd.read_csv('x.csv')" },
    ]);
  });
});
