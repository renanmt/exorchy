import { For } from "solid-js";
import { Select as ArkSelect, createListCollection } from "@ark-ui/solid/select";
import { Portal } from "solid-js/web";

interface SelectOption {
  value: string;
  label: string;
  /** Indent level for tree rendering (0 = root). */
  depth?: number;
  /** Optional override for the trigger's displayed text. Use this when
      the dropdown label is contextual (e.g. just "Baseball" under a
      "Sports" parent) but the trigger should show full context
      ("Sports / Baseball") once selected. */
  triggerLabel?: string;
  /** Render as a non-selectable section header instead of an item. */
  header?: boolean;
}

interface SelectProps {
  options: SelectOption[];
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  class?: string;
}

export function Select(props: SelectProps) {
  const collection = () =>
    createListCollection({
      // Headers are display-only rows; keeping them out of the collection
      // means keyboard navigation and value matching skip them entirely.
      items: props.options.filter((o) => !o.header),
      itemToValue: (item) => item.value,
      itemToString: (item) => item.triggerLabel ?? item.label,
    });

  return (
    <ArkSelect.Root
      class={props.class}
      collection={collection()}
      value={props.value ? [props.value] : []}
      onValueChange={(details) => {
        const val = details.value[0] ?? "";
        props.onChange(val);
      }}
      positioning={{ sameWidth: true }}
    >
      <ArkSelect.Control>
        <ArkSelect.Trigger class="ark-select-trigger">
          <ArkSelect.ValueText placeholder={props.placeholder ?? "Select..."} />
          <span class="ark-select-arrow">&#9662;</span>
        </ArkSelect.Trigger>
      </ArkSelect.Control>
      <Portal>
        <ArkSelect.Positioner>
          <ArkSelect.Content class="ark-select-content">
            <For each={props.options}>
              {(option) =>
                option.header ? (
                  <div class="ark-select-group-label">{option.label}</div>
                ) : (
                  <ArkSelect.Item
                    item={option}
                    class={`ark-select-item${option.depth ? ` depth-${option.depth}` : ""}`}
                  >
                    <ArkSelect.ItemText>{option.label}</ArkSelect.ItemText>
                    <ArkSelect.ItemIndicator class="ark-select-indicator">
                      &#10003;
                    </ArkSelect.ItemIndicator>
                  </ArkSelect.Item>
                )
              }
            </For>
          </ArkSelect.Content>
        </ArkSelect.Positioner>
      </Portal>
    </ArkSelect.Root>
  );
}
