// Pure model behind the ds Stepper (components/ds/Stepper.tsx), kept free of React Native so it
// can be unit tested in plain node.

export type StepperStatus = "done" | "current" | "todo" | "skipped";

export interface StepperStep {
  key: string;
  label: string;
  /** Explicit status; when absent it follows `activeIndex`. */
  status?: StepperStatus;
  /** Marks a step that still has to pass (rail only). */
  required?: boolean;
  /** Marks a step added since the last run (rail only). */
  isNew?: boolean;
}

/** The status of every step: explicit where given, else relative to the active one. */
export function stepperStatuses(steps: StepperStep[], activeIndex: number): StepperStatus[] {
  return steps.map((step, index) => {
    if (step.status) return step.status;
    if (index < activeIndex) return "done";
    if (index === activeIndex) return "current";
    return "todo";
  });
}
