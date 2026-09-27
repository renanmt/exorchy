import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render } from "solid-js/web";
import { invoke } from "@tauri-apps/api/core";
import type { Game } from "../api/tauri";

// The real store resolves directories from the backend at startup; a card in
// isolation just needs *some* candidate so the <img> renders.
vi.mock("../stores/thumbnails", () => ({
  thumbnailCandidates: () => ["/covers/key1.jpg"],
  bestThumbnailPath: () => "/covers/key1.jpg",
}));

const { GameCard } = await import("./GameCard");

const mockInvoke = vi.mocked(invoke);

function makeGame(over: Partial<Game> = {}): Game {
  return {
    id: 1, title: "Descent", sort_title: "Descent", platform: "MS-DOS",
    developer: null, publisher: null, release_date: null, year: 1995,
    genre: "Action", series: null, play_mode: null, rating: null,
    description: null, notes: null, source: null, application_path: null,
    dosbox_conf: null, status: null, region: null, max_players: null,
    language: "EN", shortcode: "DESCENT", torrent_source: "eXoDOS",
    in_library: false, installed: false, game_torrent_index: 1,
    gamedata_torrent_index: null, download_size: 120_000_000,
    has_thumbnail: true, dosbox_variant: null, favorited: false,
    thumbnail_key: "key1", manual_path: null, last_played: null,
    available_languages: null, ...over,
  } as Game;
}

function mount(game: Game) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <GameCard game={game} onDetail={() => {}} />, host);
  return { host, dispose };
}

describe("GameCard", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    mockInvoke.mockResolvedValue([]);
  });
  afterEach(() => { document.body.innerHTML = ""; });

  it("renders the download size when downloads are possible", async () => {
    const { host, dispose } = mount(makeGame());
    await Promise.resolve();
    expect(host.textContent).toContain("Descent");
    expect(host.textContent).toContain("120");
    dispose(); host.remove();
  });

  // jsdom has no IntersectionObserver, which exercises nearViewport.ts's
  // fallback: without observer support a card must load its cover immediately
  // rather than never showing one.
  it("still resolves a cover without IntersectionObserver support", async () => {
    expect(typeof (globalThis as any).IntersectionObserver).toBe("undefined");
    const { host, dispose } = mount(makeGame());
    await Promise.resolve();
    const img = host.querySelector("img.game-card-thumb");
    expect(img?.getAttribute("src") ?? "").toContain("asset://");
    dispose(); host.remove();
  });
});
