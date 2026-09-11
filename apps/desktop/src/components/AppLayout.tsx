import { useEffect, useState } from "react";
import { Outlet } from "react-router-dom";
import Sidebar from "./Sidebar";
import PlayerBar from "./PlayerBar";
import NowPlaying from "@/pages/NowPlaying";
import { usePlaylists } from "@/stores/playlists";
import { usePlayer } from "@/stores/player";

export default function AppLayout() {
  const loadPlaylists = usePlaylists((s) => s.loadPlaylists);
  const restore = usePlayer((s) => s.restore);
  // Full-screen "Now Playing" overlay (P7 — synced lyrics). Triggered by
  // clicking the cover in `PlayerBar`. Lives here so the same parent that
  // mounts `PlayerBar` also renders the overlay above it — no need to plumb
  // a callback through nested routes, and no extra global state.
  const [nowPlayingOpen, setNowPlayingOpen] = useState(false);

  // On launch: load persisted playlists and re-resolve the last playback
  // position (Bilibili URLs expire, so we never trust the persisted one).
  useEffect(() => {
    void loadPlaylists();
    void restore();
  }, [loadPlaylists, restore]);

  return (
    <div className="flex h-full w-full">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <main className="flex-1 overflow-y-auto px-7 py-7">
          <Outlet />
        </main>
        <PlayerBar onOpenNowPlaying={() => setNowPlayingOpen(true)} />
      </div>
      {nowPlayingOpen && (
        <NowPlaying onClose={() => setNowPlayingOpen(false)} />
      )}
    </div>
  );
}
