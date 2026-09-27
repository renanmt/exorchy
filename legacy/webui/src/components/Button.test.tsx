import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { Button } from "./Button";

/** The component exists for the states, so that is what is pinned here. */
describe("Button", () => {
  afterEach(() => { document.body.innerHTML = ""; });

  function mount(el: () => any) {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(el, host);
    return { host, dispose, btn: () => host.querySelector("button")! };
  }

  // Layout lives on the shared class so an icon never sits on the baseline.
  it("always carries the shared layout class", () => {
    const { btn, dispose } = mount(() => <Button>Go</Button>);
    expect(btn().className).toContain("app-btn");
    dispose();
  });

  it("maps a variant to the stylesheet class", () => {
    const { btn, dispose } = mount(() => <Button variant="danger">Remove</Button>);
    expect(btn().className).toBe("app-btn btn-danger");
    dispose();
  });

  it("keeps extra classes alongside the variant", () => {
    const { btn, dispose } = mount(() => <Button variant="action" class="btn-play">Play</Button>);
    expect(btn().className).toContain("game-detail-btn");
    expect(btn().className).toContain("btn-play");
    dispose();
  });

  // An action in flight must not be startable twice - this was hand-written at
  // every call site, and one of them always forgot the `disabled`.
  it("blocks input while loading", () => {
    const onClick = vi.fn();
    const { btn, dispose } = mount(() => <Button loading onClick={onClick}>Install</Button>);
    expect(btn().disabled).toBe(true);
    expect(btn().getAttribute("aria-busy")).toBe("true");
    btn().click();
    expect(onClick).not.toHaveBeenCalled();
    dispose();
  });

  it("shows a spinner and an optional loading label", () => {
    const { btn, dispose } = mount(() => <Button loading loadingLabel="Starting…">Install</Button>);
    expect(btn().querySelector(".btn-spinner")).not.toBeNull();
    expect(btn().textContent).toContain("Starting…");
    expect(btn().textContent).not.toContain("Install");
    dispose();
  });

  it("stays disabled when asked, independent of loading", () => {
    const onClick = vi.fn();
    const { btn, dispose } = mount(() => <Button disabled onClick={onClick}>Install</Button>);
    expect(btn().disabled).toBe(true);
    btn().click();
    expect(onClick).not.toHaveBeenCalled();
    dispose();
  });

  it("passes through everything else", () => {
    const { btn, dispose } = mount(() => <Button title="why not" type="submit">Go</Button>);
    expect(btn().getAttribute("title")).toBe("why not");
    expect(btn().getAttribute("type")).toBe("submit");
    dispose();
  });
});
