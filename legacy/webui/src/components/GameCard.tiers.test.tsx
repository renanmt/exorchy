/**
 * Cover-tier behaviour, kept apart from GameCard.test.tsx because it needs a
 * REACTIVE mock of stores/thumbnails: installing or removing a poster pack
 * rewrites the candidate list at runtime, and the other file's static mock
 * cannot express that.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { Game } from "../api/tauri";

// A reactive stand-in for the tier resolver: installing or removing a poster
// pack rewrites this list at runtime, which is exactly what these cases probe.
const [candidates, setCandidates] = createSignal<string[]>([]);
vi.mock("../stores/thumbnails", async () => ({
  thumbnailCandidates: () => candidates(),
  bestThumbnailPath: () => candidates()[0] ?? null,
}));

const { GameCard } = await import("./GameCard");

const mockInvoke = vi.mocked(invoke);

function makeGame(): Game {
  return {
    id: 1, title: "Descent", sort_title: "Descent", platform: "MS-DOS",
    developer: null, publisher: null, release_date: null, year: 1995,
    genre: "Action", series: null, play_mode: null, rating: null,
    description: null, notes: null, source: null, application_path: null,
    dosbox_conf: null, status: null, region: null, max_players: null,
    language: "EN", shortcode: "DESCENT", torrent_source: "eXoDOS",
    in_library: false, installed: false, game_torrent_index: 1,
    gamedata_torrent_index: null, download_size: 1, has_thumbnail: true,
    dosbox_variant: null, favorited: false, thumbnail_key: "key1",
    manual_path: null, last_played: null, available_languages: null,
  } as Game;
}

function mount() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <GameCard game={makeGame()} onDetail={() => {}} />, host);
  return { host, dispose };
}

const thumb = (host: HTMLElement) => host.querySelector("img.game-card-thumb");

describe("GameCard cover tiers", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    mockInvoke.mockResolvedValue([]);
    setCandidates(["/posters/key1.jpg", "/previews/key1.jpg"]);
  });
  afterEach(() => { document.body.innerHTML = ""; });

  /// Uninstalling the Box Art pack drops the poster tier, so a card that had
  /// already fallen through to index 1 was left pointing past the end of a
  /// one-entry list. Every tile in the grid rendered without a cover.
  it("keeps a cover when the poster tier disappears under it", async () => {
    const { host, dispose } = mount();
    // Force the fallback the 404 handler performs, so the card sits at index 1.
    thumb(host)?.dispatchEvent(new Event("error"));
    await Promise.resolve();
    expect(thumb(host)?.getAttribute("src") ?? "").toContain("previews");

    setCandidates(["/previews/key1.jpg"]);
    await Promise.resolve();

    const src = thumb(host)?.getAttribute("src") ?? "";
    expect(src, "the bundled preview must still render").toContain("previews");
    dispose(); host.remove();
  });

  /// A card painting its first cover has nothing to dissolve from. Fading it
  /// in anyway put a 350 ms ramp on every tile of every scroll.
  it("does not fade the first cover it paints", async () => {
    const { host, dispose } = mount();
    expect(host.querySelector("img.game-card-thumb-base")).toBeNull();
    expect(thumb(host)?.className).not.toContain("is-fading-in");
    dispose(); host.remove();
  });

  /// Installing a poster pack swaps blurry for sharp under the user's eyes -
  /// the one moment a cross-fade is wanted. The outgoing cover stays mounted
  /// until the replacement has decoded.
  it("cross-fades when a sharper tier replaces the cover on screen", async () => {
    setCandidates(["/previews/key1.jpg"]);
    const { host, dispose } = mount();
    thumb(host)?.dispatchEvent(new Event("load"));
    await Promise.resolve();

    setCandidates(["/posters/key1.jpg", "/previews/key1.jpg"]);
    await Promise.resolve();

    expect(thumb(host)?.getAttribute("src") ?? "").toContain("posters");
    expect(thumb(host)?.className).toContain("is-fading-in");
    expect(host.querySelector("img.game-card-thumb-base")?.getAttribute("src") ?? "")
      .toContain("previews");

    // The reveal is held back two animation frames, or the transition never
    // gets a painted start value and the swap is instant again.
    thumb(host)?.dispatchEvent(new Event("load"));
    await vi.waitFor(() => expect(thumb(host)?.className).toContain("is-loaded"));
    dispose(); host.remove();
  });

  /// Installing ONE collection's pack re-resolves every collection's tier
  /// dirs. Cards whose own cover did not change must not be flagged as
  /// loading: an unchanged src fires no second load event, so they faded to
  /// zero and stayed blank until the library was reloaded.
  it("leaves an untouched cover alone when another pack is installed", async () => {
    setCandidates(["/posters/key1.jpg", "/previews/key1.jpg"]);
    const { host, dispose } = mount();
    thumb(host)?.dispatchEvent(new Event("load"));
    await Promise.resolve();

    // Same entries, new array identity - exactly what loadThumbnailDir does.
    setCandidates(["/posters/key1.jpg", "/previews/key1.jpg"]);
    await Promise.resolve();

    expect(thumb(host)?.className).not.toContain("is-fading-in");
    expect(thumb(host)?.className).toContain("is-loaded");
    dispose(); host.remove();
  });
});
