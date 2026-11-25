import React, { useEffect, useRef, useState } from 'react';
import {
  Search,
  Film,
  Tv,
  Download,
  Shield,
  ShieldAlert,
  Server,
  Plus,
  Trash2,
  RefreshCw,
  Settings,
  Sliders,
  Upload,
  KeyRound,
  ChevronDown,
  ChevronUp,
  ListChecks,
} from 'lucide-react';

// ============================================================================
// TypeScript Interfaces
// ============================================================================

interface TmdbResult {
  id: number;
  media_type: 'movie' | 'tv';
  title: string;
  year?: number;
  overview?: string;
  poster_path?: string;
}

interface LibraryItem {
  id: number;
  tmdb_id: number;
  media_type: string;
  title: string;
  year?: number;
  poster_path?: string;
  status: string;
  root_folder?: string;
  quality_profile_id?: number | null;
}

interface SeasonInfo {
  season_number: number;
  monitored?: boolean;
  total_episodes?: number;
  available_episodes?: number;
}

interface Indexer {
  id: number;
  name: string;
  url: string;
  driver: string;
  api_key: string;
  enabled: boolean;
}

interface Release {
  indexer_name: string;
  title: string;
  download_url: string;
  size_bytes: number;
  seeders: number;
  quality: string;
  score: number;
}

interface DownloadItem {
  id: string;
  name: string;
  progress_pct: number;
  download_speed_bps: number;
  state: string;
}

interface VpnStatus {
  enabled: boolean;
  provider_name: string;
  interface: string;
  connected: boolean;
  kill_switch_active: boolean;
}

interface TmdbSettingsStatus {
  configured: boolean;
  masked_key?: string | null;
}

interface QualityProfileItem {
  quality: string;
  allowed: boolean;
  rank: number;
  max_movie_size_mb?: number | null;
  max_episode_size_mb?: number | null;
}

interface QualityProfile {
  id: number;
  name: string;
  upgrade_allowed: boolean;
  cutoff_quality: string;
  items: QualityProfileItem[];
}

type Tab = 'discover' | 'library' | 'downloads' | 'settings';
type SettingsSection = 'vpn' | 'indexers' | 'profiles' | 'tmdb';

const TABS: Tab[] = ['discover', 'library', 'downloads', 'settings'];
const FALLBACK_QUALITIES = [
  'Remux-2160p',
  'Bluray-2160p',
  'WEBDL-2160p',
  'Remux-1080p',
  'Bluray-1080p',
  'WEBDL-1080p',
  'HDTV-1080p',
  'Bluray-720p',
  'WEBDL-720p',
  'HDTV-720p',
  'WEBDL-480p',
  'SDTV',
  'Unknown',
];

// ============================================================================
// Reusable UI Helpers & Modals
// ============================================================================

function formatBytes(bytes: number): string {
  if (!bytes) return '0 B';
  const gb = bytes / 1024 ** 3;
  return gb >= 1 ? `${gb.toFixed(2)} GB` : `${(bytes / 1024 ** 2).toFixed(0)} MB`;
}

function PosterImage({
  posterPath,
  title,
}: {
  posterPath?: string;
  title: string;
}) {
  if (!posterPath) {
    return (
      <div className="w-full aspect-[2/3] bg-slate-800 flex items-center justify-center text-slate-500 text-xs">
        No Poster
      </div>
    );
  }

  return (
    <img
      src={`https://image.tmdb.org/t/p/w342${posterPath}`}
      alt={title}
      className="w-full aspect-[2/3] object-cover"
    />
  );
}

interface ReleaseModalProps {
  title: string | null;
  releases: Release[];
  searching: boolean;
  error: string | null;
  availableQualities: string[];
  onClose: () => void;
  onGrab: (release: Release) => void;
}

function ReleaseModal({
  title,
  releases,
  searching,
  error,
  availableQualities,
  onClose,
  onGrab,
}: ReleaseModalProps) {
  const [selectedQuality, setSelectedQuality] = useState<string>('ALL');

  useEffect(() => {
    setSelectedQuality('ALL');
  }, [title]);

  if (!title) return null;

  const qualityOptions = Array.from(
    new Set([
      ...availableQualities,
      ...releases.map((r) => r.quality).filter(Boolean),
    ])
  );

  const filteredReleases =
    selectedQuality === 'ALL'
      ? releases
      : releases.filter(
          (r) => r.quality.toLowerCase() === selectedQuality.toLowerCase()
        );

  return (
    <div className="fixed inset-0 bg-black/75 z-50 flex items-center justify-center p-4">
      <div className="bg-slate-900 border border-slate-700 rounded-2xl max-w-3xl w-full max-h-[80vh] flex flex-col">
        <div className="p-4 border-b border-slate-800 flex flex-wrap items-center justify-between gap-3">
          <h3 className="font-bold text-sm">Torrents for "{title}"</h3>

          <div className="flex items-center gap-3">
            <label className="flex items-center gap-2 text-xs text-slate-300">
              <span>Desired Quality:</span>
              <select
                value={selectedQuality}
                onChange={(e) => setSelectedQuality(e.target.value)}
                className="bg-slate-950 border border-slate-700 rounded-lg px-2.5 py-1 text-xs text-slate-100"
              >
                <option value="ALL">All Qualities ({releases.length})</option>
                {qualityOptions.map((q) => (
                  <option key={q} value={q}>
                    {q}
                  </option>
                ))}
              </select>
            </label>

            <button
              type="button"
              onClick={onClose}
              className="text-slate-400 hover:text-white text-sm"
            >
              Close
            </button>
          </div>
        </div>

        <div className="p-4 overflow-y-auto space-y-2">
          {searching ? (
            <div className="text-center py-8 text-slate-400 text-sm">
              Searching indexers...
            </div>
          ) : error ? (
            <div className="text-center py-8 text-rose-400 text-sm">
              {error}
            </div>
          ) : releases.length === 0 ? (
            <div className="text-center py-8 text-slate-400 text-sm">
              No releases found (check your indexers).
            </div>
          ) : filteredReleases.length === 0 ? (
            <div className="text-center py-8 text-slate-400 text-sm space-y-2">
              <p>No releases matched "{selectedQuality}".</p>
              <button
                type="button"
                onClick={() => setSelectedQuality('ALL')}
                className="text-indigo-400 hover:underline text-xs"
              >
                Show all {releases.length} releases
              </button>
            </div>
          ) : (
            filteredReleases.map((rel, i) => (
              <div
                key={`${rel.download_url}-${i}`}
                className="bg-slate-950 border border-slate-800 p-3 rounded-lg flex items-center justify-between gap-4"
              >
                <div className="min-w-0">
                  <div className="text-sm font-medium truncate">{rel.title}</div>
                  <div className="text-xs text-slate-400 mt-0.5 flex flex-wrap gap-3">
                    <span className="text-indigo-400 font-semibold">
                      {rel.indexer_name}
                    </span>
                    <span className="px-1.5 py-0.5 rounded bg-slate-800 text-slate-200 font-medium">
                      {rel.quality}
                    </span>
                    <span>{formatBytes(rel.size_bytes)}</span>
                    <span className="text-emerald-400">
                      {rel.seeders} seeders
                    </span>
                  </div>
                </div>
                <button
                  type="button"
                  onClick={() => onGrab(rel)}
                  className="bg-indigo-600 hover:bg-indigo-500 px-3 py-1.5 rounded-lg text-xs font-semibold shrink-0 flex items-center gap-1"
                >
                  <Download className="w-3.5 h-3.5" /> Grab
                </button>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

function DiscoverMediaCard({
  item,
  profiles,
  inLibrary,
  onRequest,
}: {
  item: TmdbResult;
  profiles: QualityProfile[];
  inLibrary: boolean;
  onRequest: (
    item: TmdbResult,
    seasons: number[] | undefined,
    profileId: number | null
  ) => Promise<void>;
}) {
  const isTv = item.media_type === 'tv';
  const [expanded, setExpanded] = useState(false);
  const [profileId, setProfileId] = useState<number | null>(
    profiles[0]?.id ?? null
  );
  const [availableSeasons, setAvailableSeasons] = useState<number[]>([
    1, 2, 3, 4, 5, 6, 7, 8,
  ]);
  const [selectedSeasons, setSelectedSeasons] = useState<Set<number>>(
    new Set([1, 2, 3, 4, 5, 6, 7, 8])
  );
  const [loadingSeasons, setLoadingSeasons] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (inLibrary) {
      setExpanded(false);
    }
  }, [inLibrary]);

  useEffect(() => {
    if (profileId === null && profiles[0]?.id != null) {
      setProfileId(profiles[0].id);
    }
  }, [profiles, profileId]);

  useEffect(() => {
    if (!isTv || !expanded || inLibrary) return;
    let cancelled = false;
    setLoadingSeasons(true);

    (async () => {
      try {
        const res = await fetch(`/api/tv/${item.id}/seasons`);
        if (res.ok) {
          const raw = await res.json();
          const list = Array.isArray(raw) ? raw : raw?.seasons ?? [];
          const nums = list
            .map((s: { season_number?: number }) => Number(s.season_number))
            .filter((n: number) => Number.isFinite(n) && n > 0)
            .sort((a: number, b: number) => a - b);
          if (!cancelled && nums.length > 0) {
            setAvailableSeasons(nums);
            setSelectedSeasons(new Set(nums));
            setLoadingSeasons(false);
            return;
          }
        }
      } catch {}

      if (!cancelled) {
        const fallback = [1, 2, 3, 4, 5, 6, 7, 8];
        setAvailableSeasons(fallback);
        setSelectedSeasons(new Set(fallback));
        setLoadingSeasons(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [isTv, expanded, inLibrary, item.id]);

  const toggleSeason = (num: number) => {
    setSelectedSeasons((prev) => {
      const next = new Set(prev);
      if (next.has(num)) next.delete(num);
      else next.add(num);
      return next;
    });
  };

  const handlePrimaryAction = async () => {
    if (inLibrary) return;

    if (isTv && !expanded) {
      setExpanded(true);
      return;
    }

    setSubmitting(true);
    try {
      const seasonsArg = isTv
        ? Array.from(selectedSeasons).sort((a, b) => a - b)
        : undefined;
      await onRequest(item, seasonsArg, profileId);
      setExpanded(false);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="bg-slate-900 border border-slate-800 rounded-xl overflow-hidden flex flex-col justify-between">
      <div>
        <PosterImage posterPath={item.poster_path} title={item.title} />
        <div className="p-3">
          <div className="flex items-center gap-1.5 text-xs text-indigo-400 uppercase font-semibold mb-1">
            {isTv ? (
              <Tv className="w-3.5 h-3.5" />
            ) : (
              <Film className="w-3.5 h-3.5" />
            )}
            {item.media_type} {item.year ? `• ${item.year}` : ''}
          </div>
          <h3 className="font-semibold text-sm line-clamp-1">{item.title}</h3>
        </div>
      </div>

      <div className="p-3 pt-0 space-y-2">
        {inLibrary ? (
          <div className="w-full bg-emerald-950/70 border border-emerald-800/80 text-emerald-400 text-xs font-semibold py-2 px-2.5 rounded-lg text-center">
            In Library
          </div>
        ) : (
          <>
            {expanded && (
              <div className="bg-slate-950 border border-slate-800 rounded-lg p-2.5 space-y-2.5 text-xs">
                {profiles.length > 0 && (
                  <div>
                    <label className="block text-[11px] text-slate-400 mb-1">
                      Desired Quality
                    </label>
                    <select
                      value={profileId ?? ''}
                      onChange={(e) =>
                        setProfileId(
                          e.target.value ? Number(e.target.value) : null
                        )
                      }
                      className="w-full bg-slate-900 border border-slate-700 rounded px-2 py-1 text-xs text-slate-100"
                    >
                      {profiles.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.name}
                        </option>
                      ))}
                    </select>
                  </div>
                )}

                {isTv && (
                  <div>
                    <div className="flex items-center justify-between text-[11px] text-slate-400 mb-1.5">
                      <span>Seasons</span>
                      <div className="flex gap-2">
                        <button
                          type="button"
                          onClick={() =>
                            setSelectedSeasons(new Set(availableSeasons))
                          }
                          className="text-indigo-400 hover:underline"
                        >
                          All
                        </button>
                        <button
                          type="button"
                          onClick={() => setSelectedSeasons(new Set())}
                          className="text-slate-400 hover:underline"
                        >
                          None
                        </button>
                      </div>
                    </div>

                    {loadingSeasons ? (
                      <p className="text-[11px] text-slate-500">
                        Loading seasons...
                      </p>
                    ) : (
                      <div className="grid grid-cols-3 gap-1.5 max-h-28 overflow-y-auto pr-0.5">
                        {availableSeasons.map((num) => {
                          const checked = selectedSeasons.has(num);
                          return (
                            <label
                              key={num}
                              className={`flex items-center gap-1.5 px-1.5 py-1 rounded border cursor-pointer text-[11px] ${
                                checked
                                  ? 'bg-indigo-600/25 border-indigo-500/60 text-indigo-200'
                                  : 'bg-slate-900 border-slate-800 text-slate-400'
                              }`}
                            >
                              <input
                                type="checkbox"
                                checked={checked}
                                onChange={() => toggleSeason(num)}
                                className="accent-indigo-500 rounded"
                              />
                              <span>S{String(num).padStart(2, '0')}</span>
                            </label>
                          );
                        })}
                      </div>
                    )}
                  </div>
                )}
              </div>
            )}

            <div className="flex gap-1.5">
              <button
                type="button"
                disabled={submitting}
                onClick={handlePrimaryAction}
                className="flex-1 bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-xs font-semibold py-2 px-2.5 rounded-lg transition"
              >
                {submitting
                  ? 'Requesting...'
                  : isTv && expanded
                  ? 'Confirm Request'
                  : '+ Request'}
              </button>

              <button
                type="button"
                onClick={() => setExpanded((prev) => !prev)}
                aria-label="Toggle request options"
                className="px-2 py-2 rounded-lg text-xs bg-slate-800 hover:bg-slate-700 text-slate-300 transition"
              >
                {expanded ? (
                  <ChevronUp className="w-3.5 h-3.5" />
                ) : (
                  <ChevronDown className="w-3.5 h-3.5" />
                )}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function LibraryTvSeasonsMenu({
  item,
  notify,
  onRefreshLibrary,
}: {
  item: LibraryItem;
  notify: (msg: string) => void;
  onRefreshLibrary: () => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [seasons, setSeasons] = useState<SeasonInfo[]>([]);
  const [savingSeason, setSavingSeason] = useState<number | null>(null);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setLoading(true);

    (async () => {
      try {
        const res = await fetch(`/api/library/${item.id}/seasons`);
        if (res.ok) {
          const raw = await res.json();
          const list: SeasonInfo[] = Array.isArray(raw) ? raw : raw?.seasons ?? [];
          const filtered = list
            .filter((s) => Number(s.season_number) > 0)
            .sort((a, b) => Number(a.season_number) - Number(b.season_number));
          if (!cancelled) {
            setSeasons(filtered);
          }
        } else if (!cancelled) {
          setSeasons([]);
        }
      } catch {
        if (!cancelled) {
          setSeasons([]);
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [open, item.id]);

  const handleToggleSeason = async (seasonNum: number, nextMonitored: boolean) => {
    setSavingSeason(seasonNum);
    const prevSeasons = seasons;
    setSeasons((cur) =>
      cur.map((s) =>
        s.season_number === seasonNum ? { ...s, monitored: nextMonitored } : s
      )
    );

    try {
      const res = await fetch(
        `/api/library/${item.id}/seasons/${seasonNum}/monitor`,
        {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ monitored: nextMonitored }),
        }
      );

      if (res.ok) {
        notify(
          `${nextMonitored ? 'Enabled' : 'Disabled'} Season ${seasonNum} for "${item.title}"`
        );
        await onRefreshLibrary();
      } else {
        setSeasons(prevSeasons);
        notify(`Failed to update Season ${seasonNum}`);
      }
    } catch {
      setSeasons(prevSeasons);
      notify(`Failed to update Season ${seasonNum}`);
    } finally {
      setSavingSeason(null);
    }
  };

  return (
    <div className="mt-2">
      <button
        type="button"
        onClick={() => setOpen((prev) => !prev)}
        className="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg bg-slate-950 hover:bg-slate-800 border border-slate-700 text-xs font-medium text-slate-200 transition"
      >
        <span className="flex items-center gap-1.5">
          <ListChecks className="w-3.5 h-3.5 text-indigo-400" />
          Select seasons
        </span>
        {open ? (
          <ChevronUp className="w-3.5 h-3.5 text-slate-400" />
        ) : (
          <ChevronDown className="w-3.5 h-3.5 text-slate-400" />
        )}
      </button>

      {open && (
        <div className="mt-1.5 bg-slate-950 border border-slate-800 rounded-lg p-2 space-y-1 max-h-40 overflow-y-auto">
          {loading ? (
            <p className="text-[11px] text-slate-500 px-1 py-1">
              Loading seasons...
            </p>
          ) : seasons.length === 0 ? (
            <p className="text-[11px] text-slate-500 px-1 py-1">
              No seasons found in database.
            </p>
          ) : (
            seasons.map((s) => {
              const isChecked = s.monitored ?? true;
              const isSaving = savingSeason === s.season_number;
              return (
                <label
                  key={s.season_number}
                  className="flex items-center justify-between gap-2 px-1.5 py-1 rounded hover:bg-slate-900 cursor-pointer text-xs"
                >
                  <span className="flex items-center gap-2 text-slate-200">
                    <input
                      type="checkbox"
                      checked={isChecked}
                      disabled={isSaving}
                      onChange={(e) =>
                        handleToggleSeason(s.season_number, e.target.checked)
                      }
                      className="accent-indigo-500 rounded"
                    />
                    <span>Season {s.season_number}</span>
                  </span>

                  {typeof s.available_episodes === 'number' &&
                    typeof s.total_episodes === 'number' && (
                      <span className="text-[10px] text-slate-400">
                        {s.available_episodes}/{s.total_episodes}
                      </span>
                    )}
                </label>
              );
            })
          )}
        </div>
      )}
    </div>
  );
}

// ============================================================================
// Main Application Component
// ============================================================================

export default function App() {
  const [tab, setTab] = useState<Tab>('discover');
  const [settingsSection, setSettingsSection] = useState<SettingsSection>('vpn');
  const [vpn, setVpn] = useState<VpnStatus | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  // Discover state
  const [query, setQuery] = useState('');
  const [searchResults, setSearchResults] = useState<TmdbResult[]>([]);
  const [searching, setSearching] = useState(false);

  // Release modal state
  const [releases, setReleases] = useState<Release[]>([]);
  const [releaseModalTitle, setReleaseModalTitle] = useState<string | null>(null);
  const [activeReleaseMediaId, setActiveReleaseMediaId] = useState<number | null>(null);
  const [searchingReleases, setSearchingReleases] = useState(false);
  const [releaseSearchError, setReleaseSearchError] = useState<string | null>(null);

  // Library & top-nav scan state
  const [library, setLibrary] = useState<LibraryItem[]>([]);
  const [scanning, setScanning] = useState(false);

  // Indexers state
  const [indexers, setIndexers] = useState<Indexer[]>([]);
  const [editingIndexerId, setEditingIndexerId] = useState<number | null>(null);
  const [idxName, setIdxName] = useState('');
  const [idxUrl, setIdxUrl] = useState('');
  const [idxDriver, setIdxDriver] = useState('apibay');
  const [idxApiKey, setIdxApiKey] = useState('');
  const [idxEnabled, setIdxEnabled] = useState(true);

  // Downloads state
  const [downloads, setDownloads] = useState<DownloadItem[]>([]);
  const prevDownloadIdsRef = useRef<Set<string>>(new Set());

  // VPN form state
  const [vpnEnabled, setVpnEnabled] = useState(true);
  const [vpnProvider, setVpnProvider] = useState('NordVPN');
  const [wgConfig, setWgConfig] = useState('');

  // Custom TMDB API key state
  const [tmdbStatus, setTmdbStatus] = useState<TmdbSettingsStatus | null>(null);
  const [tmdbApiKeyInput, setTmdbApiKeyInput] = useState('');
  const [savingTmdb, setSavingTmdb] = useState(false);

  // Quality Profiles state
  const [availableQualities, setAvailableQualities] = useState<string[]>(FALLBACK_QUALITIES);
  const [profiles, setProfiles] = useState<QualityProfile[]>([]);
  const [editingProfileId, setEditingProfileId] = useState<number | 'new'>('new');
  const [profileName, setProfileName] = useState('');
  const [upgradeAllowed, setUpgradeAllowed] = useState(false);
  const [cutoffQuality, setCutoffQuality] = useState('WEBDL-1080p');
  const [profileItems, setProfileItems] = useState<QualityProfileItem[]>([]);

  const notify = (msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 4000);
  };

  const buildDefaultProfileItems = (qualities: string[]): QualityProfileItem[] =>
    qualities.map((q, idx) => ({
      quality: q,
      allowed: idx < 5,
      rank: qualities.length - idx,
      max_movie_size_mb: null,
      max_episode_size_mb: null,
    }));

  const resetProfileForm = (qualities: string[] = availableQualities) => {
    setEditingProfileId('new');
    setProfileName('');
    setUpgradeAllowed(false);
    setCutoffQuality(qualities[5] ?? qualities[0] ?? 'WEBDL-1080p');
    setProfileItems(buildDefaultProfileItems(qualities));
  };

  const startEditingProfile = (profile: QualityProfile) => {
    setEditingProfileId(profile.id);
    setProfileName(profile.name);
    setUpgradeAllowed(profile.upgrade_allowed);
    setCutoffQuality(profile.cutoff_quality);

    const existingByName = new Map(profile.items.map((i) => [i.quality, i]));
    const merged: QualityProfileItem[] = availableQualities.map((q, idx) => {
      const found = existingByName.get(q);
      return (
        found ?? {
          quality: q,
          allowed: false,
          rank: availableQualities.length - idx,
          max_movie_size_mb: null,
          max_episode_size_mb: null,
        }
      );
    });
    setProfileItems(merged);
  };

  // ============================================================================
  // Data Fetching
  // ============================================================================

  const fetchVpn = async () => {
    try {
      const res = await fetch('/api/vpn');
      if (res.ok) setVpn(await res.json());
    } catch {}
  };

  const fetchTmdbSettings = async () => {
    try {
      const res = await fetch('/api/settings/tmdb');
      if (res.ok) setTmdbStatus(await res.json());
    } catch {}
  };

  const fetchLibrary = async () => {
    const res = await fetch('/api/library');
    if (res.ok) setLibrary(await res.json());
  };

  const fetchIndexers = async () => {
    const res = await fetch('/api/indexers');
    if (res.ok) setIndexers(await res.json());
  };

  const fetchQualitiesAndProfiles = async () => {
    let qs = FALLBACK_QUALITIES;
    try {
      const qRes = await fetch('/api/qualities');
      if (qRes.ok) {
        const parsed: string[] = await qRes.json();
        if (parsed.length > 0) qs = parsed;
      }
    } catch {}
    setAvailableQualities(qs);

    try {
      const pRes = await fetch('/api/quality-profiles');
      if (pRes.ok) {
        const list: QualityProfile[] = await pRes.json();
        setProfiles(list);
        if (list.length > 0 && editingProfileId === 'new') {
          const first = list[0];
          setEditingProfileId(first.id);
          setProfileName(first.name);
          setUpgradeAllowed(first.upgrade_allowed);
          setCutoffQuality(first.cutoff_quality);

          const existingByName = new Map(first.items.map((i) => [i.quality, i]));
          setProfileItems(
            qs.map((q, idx) => {
              const found = existingByName.get(q);
              return (
                found ?? {
                  quality: q,
                  allowed: false,
                  rank: qs.length - idx,
                  max_movie_size_mb: null,
                  max_episode_size_mb: null,
                }
              );
            })
          );
        }
      }
    } catch {}

    setProfileItems((prev) => (prev.length > 0 ? prev : buildDefaultProfileItems(qs)));
  };

  const fetchDownloads = async () => {
    const res = await fetch('/api/downloads');
    if (!res.ok) return;

    const nextDownloads: DownloadItem[] = await res.json();
    const nextIds = new Set(nextDownloads.map((d) => d.id));
    const prevIds = prevDownloadIdsRef.current;

    const anyRemoved = Array.from(prevIds).some((id) => !nextIds.has(id));
    prevDownloadIdsRef.current = nextIds;
    setDownloads(nextDownloads);

    if (anyRemoved) {
      fetchLibrary();
    }
  };

  const [runningWantedScan, setRunningWantedScan] = useState(false);

  const handleRunWantedScan = async () => {
    if (runningWantedScan) return;
    setRunningWantedScan(true);
    try {
      const res = await fetch('/api/wanted/search', {
        method: 'POST',
      });
      if (!res.ok) {
        const errText = await res.text();
        notify(`Wanted scan failed (${res.status}): ${errText || 'Check backend route'}`);
        return;
      }
      notify('Wanted scan complete');
      await Promise.allSettled([fetchDownloads(), fetchLibrary()]);
    } catch (err) {
      console.error('Failed to run wanted scan:', err);
      notify('Wanted scan request failed');
    } finally {
      setRunningWantedScan(false);
    }
  };

  useEffect(() => {
    fetchVpn();
    fetchTmdbSettings();
    fetchLibrary();
    fetchIndexers();
    fetchQualitiesAndProfiles();
    fetchDownloads();

    const interval = setInterval(() => {
      fetchDownloads();
      fetchVpn();
    }, 2500);

    return () => clearInterval(interval);
  }, []);

  // ============================================================================
  // Action Handlers
  // ============================================================================

  const handleScanLibrary = async () => {
    if (scanning) return;
    setScanning(true);
    try {
      const res = await fetch('/api/scan', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
      });
      if (res.ok) {
        const summary = await res.json();
        notify(
          `Library scan complete: ${summary.files_found} files found, ${summary.imported_or_updated} imported/updated`
        );
        await fetchLibrary();
      } else {
        notify(`Scan failed: ${await res.text()}`);
      }
    } catch {
      notify('Scan request failed');
    } finally {
      setScanning(false);
    }
  };

  const handleSearch = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!query.trim()) return;
    setSearching(true);
    const res = await fetch(`/api/search?q=${encodeURIComponent(query)}`);
    if (res.ok) {
      setSearchResults(await res.json());
    } else {
      notify(`Search error: ${await res.text()}`);
    }
    setSearching(false);
  };

  const sendMediaRequest = async (
    item: TmdbResult,
    seasons?: number[],
    qualityProfileId?: number | null
  ) => {
    const res = await fetch('/api/requests', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        tmdb_id: item.id,
        media_type: item.media_type,
        title: item.title,
        year: item.year,
        overview: item.overview,
        poster_path: item.poster_path,
        quality_profile_id: qualityProfileId ?? profiles[0]?.id ?? 1,
        ...(seasons ? { seasons } : {}),
      }),
    });
    if (res.ok) {
      const data = await res.json();
      const seasonNote =
        item.media_type === 'tv' && seasons
          ? ` — Seasons ${seasons.join(', ')}`
          : '';
      notify(`Requested "${item.title}"${seasonNote} (${data.status})`);
      await fetchLibrary();
    } else {
      notify(`Request failed: ${await res.text()}`);
    }
  };

  const handleSearchReleases = async (
    title: string,
    year?: number,
    mediaItemId?: number
  ) => {
    const q = year ? `${title} ${year}` : title;
    setReleaseModalTitle(q);
    setActiveReleaseMediaId(mediaItemId ?? null);
    setReleases([]);
    setReleaseSearchError(null);
    setSearchingReleases(true);

    try {
      const res = await fetch(`/api/releases?q=${encodeURIComponent(q)}`);
      if (res.ok) {
        const data = await res.json();
        setReleases(Array.isArray(data) ? data : []);
      } else {
        const errText = await res.text();
        setReleaseSearchError(errText || `Search failed (${res.status})`);
      }
    } catch {
      setReleaseSearchError('Failed to reach release search endpoint');
    } finally {
      setSearchingReleases(false);
    }
  };

  const handleGrabRelease = async (rel: Release) => {
    const res = await fetch('/api/downloads', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        title: rel.title,
        download_url: rel.download_url,
        media_item_id: activeReleaseMediaId,
      }),
    });
    if (res.ok) {
      notify(`Started torrent: ${rel.title}`);
      setReleaseModalTitle(null);
      setActiveReleaseMediaId(null);
      setSearchingReleases(false);
      setReleaseSearchError(null);
      setTab('downloads');
      fetchDownloads();
    } else {
      const err = await res.text();
      notify(`Error: ${err}`);
    }
  };

  const handleChangeLibraryProfile = async (
    mediaId: number,
    nextProfileId: number | null
  ) => {
    const res = await fetch(`/api/library/${mediaId}/quality-profile`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ quality_profile_id: nextProfileId }),
    });
    if (res.ok) {
      setLibrary((prev) =>
        prev.map((m) =>
          m.id === mediaId ? { ...m, quality_profile_id: nextProfileId } : m
        )
      );
      notify('Updated desired quality profile');
    } else {
      notify('Failed to update quality profile');
    }
  };

  // Confirms before deleting a media item and all associated files via DELETE /api/library/:id
  const handleDeleteLibraryItem = async (
    e: React.MouseEvent<HTMLButtonElement>,
    item: LibraryItem
  ) => {
    e.stopPropagation();

    const confirmed = window.confirm(
      `Delete "${item.title}"${item.year ? ` (${item.year})` : ''} and all associated files? This cannot be undone.`
    );
    if (!confirmed) return;

    const res = await fetch(`/api/library/${item.id}`, {
      method: 'DELETE',
    });

    if (res.ok) {
      setLibrary((prev) => prev.filter((m) => m.id !== item.id));
      notify(`Deleted "${item.title}" and associated files`);
    } else {
      const err = await res.text();
      notify(`Delete failed: ${err || res.statusText}`);
    }
  };

  const resetIndexerForm = () => {
    setEditingIndexerId(null);
    setIdxName('');
    setIdxUrl('');
    setIdxDriver('apibay');
    setIdxApiKey('');
    setIdxEnabled(true);
  };

  const startEditingIndexer = (idx: Indexer) => {
    setEditingIndexerId(idx.id);
    setIdxName(idx.name);
    setIdxUrl(idx.url);
    setIdxDriver(idx.driver || 'apibay');
    setIdxApiKey(idx.api_key || '');
    setIdxEnabled(idx.enabled ?? true);
  };

  const handleSaveIndexer = async (e: React.FormEvent) => {
    e.preventDefault();

    const payload = {
      name: idxName.trim(),
      url: idxUrl.trim(),
      driver: idxDriver,
      api_key: idxApiKey.trim(),
      enabled: idxEnabled,
    };

    const isEditing = editingIndexerId !== null;
    const url = isEditing ? `/api/indexers/${editingIndexerId}` : '/api/indexers';
    const method = isEditing ? 'PUT' : 'POST';

    const res = await fetch(url, {
      method,
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });

    if (res.ok) {
      resetIndexerForm();
      await fetchIndexers();
      notify(isEditing ? 'Indexer updated' : 'Indexer added');
    } else {
      notify(`Indexer error: ${await res.text()}`);
    }
  };

  const handleDeleteIndexer = async (id: number) => {
    await fetch(`/api/indexers/${id}`, { method: 'DELETE' });
    if (editingIndexerId === id) {
      resetIndexerForm();
    }
    fetchIndexers();
  };

  const handleDeleteDownload = async (id: string) => {
    await fetch(`/api/downloads/${id}`, { method: 'DELETE' });
    fetchDownloads();
  };

  const handleWgFileUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const text = await file.text();
    setWgConfig(text);
    const baseName = file.name.replace(/\.conf$/i, '');
    if (baseName && vpnProvider === 'NordVPN') {
      setVpnProvider(baseName);
    }
    notify(`Loaded WireGuard config from ${file.name}`);
  };

  const handleSaveVpn = async (e: React.FormEvent) => {
    e.preventDefault();
    const res = await fetch('/api/vpn', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        enabled: vpnEnabled,
        provider_name: vpnProvider,
        wg_config: wgConfig,
      }),
    });
    if (res.ok) {
      setVpn(await res.json());
      notify('WireGuard configuration updated');
    } else {
      notify(`VPN Error: ${await res.text()}`);
    }
  };

  const handleSaveTmdbKey = async (e: React.FormEvent) => {
    e.preventDefault();
    setSavingTmdb(true);
    try {
      const res = await fetch('/api/settings/tmdb', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ api_key: tmdbApiKeyInput }),
      });
      if (res.ok) {
        const updated: TmdbSettingsStatus = await res.json();
        setTmdbStatus(updated);
        setTmdbApiKeyInput('');
        notify(
          updated.configured
            ? 'Custom TMDB API key saved'
            : 'Custom TMDB key cleared (using environment fallback)'
        );
      } else {
        notify(`TMDB settings error: ${await res.text()}`);
      }
    } finally {
      setSavingTmdb(false);
    }
  };

  const handleSaveProfile = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!profileName.trim()) {
      notify('Please enter a profile name');
      return;
    }

    const payload = {
      name: profileName.trim(),
      upgrade_allowed: upgradeAllowed,
      cutoff_quality: cutoffQuality,
      items: profileItems,
    };

    const isNew = editingProfileId === 'new';
    const url = isNew
      ? '/api/quality-profiles'
      : `/api/quality-profiles/${editingProfileId}`;
    const method = isNew ? 'POST' : 'PUT';

    const res = await fetch(url, {
      method,
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });

    if (res.ok) {
      await fetchQualitiesAndProfiles();
      notify(isNew ? 'Quality profile created' : 'Quality profile updated');
      if (isNew) resetProfileForm();
    } else {
      notify(`Profile error: ${await res.text()}`);
    }
  };

  const vpnBadgeClass = vpn?.enabled
    ? vpn.connected
      ? 'bg-emerald-950/60 border-emerald-700 text-emerald-400'
      : 'bg-rose-950/60 border-rose-700 text-rose-400'
    : 'bg-slate-800 border-slate-700 text-slate-400';

  const vpnBadgeLabel = vpn?.enabled
    ? vpn.connected
      ? `VPN: ${vpn.provider_name} (${vpn.interface})`
      : 'VPN DISCONNECTED (KILL SWITCH)'
    : 'VPN Off';

  // ============================================================================
  // Render
  // ============================================================================

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100">
      {toast && (
        <div className="fixed bottom-5 right-5 z-50 bg-indigo-600 text-white px-4 py-3 rounded-lg shadow-xl text-sm font-medium">
          {toast}
        </div>
      )}

      {/* Top Navigation Bar with Wanted Scan & Scan Library buttons */}
      <header className="border-b border-slate-800 bg-slate-900/80 backdrop-blur sticky top-0 z-30">
        <div className="max-w-6xl mx-auto px-4 h-16 flex items-center justify-between gap-4">
          <div className="flex items-center gap-6">
            <span className="text-xl font-bold tracking-tight text-indigo-400">
              🪝 Hook
            </span>
            <nav className="flex gap-1">
              {TABS.map((t) => (
                <button
                  key={t}
                  type="button"
                  onClick={() => setTab(t)}
                  className={`px-3 py-1.5 rounded-md text-sm font-medium capitalize transition flex items-center gap-1.5 ${
                    tab === t
                      ? 'bg-indigo-600 text-white'
                      : 'text-slate-400 hover:text-white hover:bg-slate-800'
                  }`}
                >
                  {t === 'settings' && <Settings className="w-3.5 h-3.5" />}
                  {t}
                  {t === 'downloads' && downloads.length > 0 && (
                    <span className="ml-1 px-1.5 py-0.5 text-xs bg-indigo-500/30 rounded-full">
                      {downloads.length}
                    </span>
                  )}
                </button>
              ))}
            </nav>
          </div>

          <div className="flex items-center gap-2.5">
            <button
              type="button"
              onClick={handleRunWantedScan}
              disabled={runningWantedScan}
              className="bg-slate-800 hover:bg-slate-700 border border-slate-700 px-3 py-1.5 rounded-lg text-xs font-semibold text-slate-100 flex items-center gap-1.5 transition disabled:opacity-60"
            >
              <RefreshCw
                className={`w-3.5 h-3.5 text-indigo-300 ${runningWantedScan ? 'animate-spin' : ''}`}
              />
              {runningWantedScan ? 'Searching Wanted...' : 'Find Wanted'}
            </button>

            <button
              type="button"
              onClick={handleScanLibrary}
              disabled={scanning}
              className="bg-slate-800 hover:bg-slate-700 border border-slate-700 px-3 py-1.5 rounded-lg text-xs font-semibold text-slate-100 flex items-center gap-1.5 transition disabled:opacity-60"
            >
              <RefreshCw
                className={`w-3.5 h-3.5 text-indigo-400 ${scanning ? 'animate-spin' : ''}`}
              />
              {scanning ? 'Scanning...' : 'Scan Library'}
            </button>

            <button
              type="button"
              onClick={() => {
                setTab('settings');
                setSettingsSection('vpn');
              }}
              className={`flex items-center gap-2 px-3 py-1.5 rounded-full text-xs font-semibold border ${vpnBadgeClass}`}
            >
              {vpn?.enabled && vpn.connected ? (
                <Shield className="w-3.5 h-3.5" />
              ) : (
                <ShieldAlert className="w-3.5 h-3.5" />
              )}
              {vpnBadgeLabel}
            </button>
          </div>
        </div>
      </header>

      <main className="max-w-6xl mx-auto px-4 py-8">
        {/* ---------------- DISCOVER ---------------- */}
        {tab === 'discover' && (
          <div>
            <form onSubmit={handleSearch} className="flex gap-3 mb-8">
              <div className="relative flex-1">
                <Search className="w-5 h-5 absolute left-3.5 top-3 text-slate-400" />
                <input
                  type="text"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="Search movies & TV shows on TMDB..."
                  className="w-full bg-slate-900 border border-slate-700 rounded-lg pl-11 pr-4 py-2.5 text-slate-100 focus:outline-none focus:border-indigo-500"
                />
              </div>
              <button
                type="submit"
                disabled={searching}
                className="bg-indigo-600 hover:bg-indigo-500 px-6 py-2.5 rounded-lg font-medium transition"
              >
                {searching ? 'Searching...' : 'Search'}
              </button>
            </form>

            <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-5">
              {searchResults.map((item) => {
                const inLibrary = library.some(
                  (libItem) =>
                    libItem.tmdb_id === item.id && libItem.media_type === item.media_type
                );

                return (
                  <DiscoverMediaCard
                    key={`${item.media_type}-${item.id}`}
                    item={item}
                    profiles={profiles}
                    inLibrary={inLibrary}
                    onRequest={sendMediaRequest}
                  />
                );
              })}
            </div>
          </div>
        )}

        {/* ---------------- LIBRARY ---------------- */}
        {tab === 'library' && (
          <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-5">
            {library.map((item) => {
              const statusClass =
                item.status === 'available'
                  ? 'bg-emerald-950 text-emerald-400 border-emerald-800'
                  : item.status === 'partial'
                  ? 'bg-amber-950 text-amber-400 border-amber-800'
                  : 'bg-indigo-950 text-indigo-400 border-indigo-800';

              const isTv = item.media_type === 'tv';

              return (
                <div
                  key={item.id}
                  className="group relative bg-slate-900 border border-slate-800 hover:border-slate-700 rounded-xl overflow-hidden flex flex-col justify-between transition"
                >
                  {/* Hover trash button in the top-right corner */}
                  <button
                    type="button"
                    onClick={(e) => handleDeleteLibraryItem(e, item)}
                    title={`Delete ${item.title}`}
                    aria-label={`Delete ${item.title}`}
                    className="
                      absolute top-2.5 right-2.5 z-10 p-2 rounded-lg
                      bg-slate-950/85 border border-slate-700 text-slate-200
                      opacity-0 group-hover:opacity-100 focus:opacity-100
                      hover:bg-rose-600 hover:border-rose-500 hover:text-white
                      transition-all duration-150
                    "
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>

                  <div>
                    <PosterImage
                      posterPath={item.poster_path}
                      title={item.title}
                    />
                    <div className="p-3">
                      <div className="flex items-center justify-between mb-1">
                        <span className="text-xs text-slate-400 uppercase">
                          {item.media_type}
                        </span>
                        <span
                          className={`text-[11px] px-2 py-0.5 rounded-full font-semibold border ${statusClass}`}
                        >
                          {item.status}
                        </span>
                      </div>
                      <h3 className="font-semibold text-sm line-clamp-1">
                        {item.title}
                      </h3>

                      {profiles.length > 0 && (
                        <div className="mt-2">
                          <label className="block text-[11px] text-slate-400 mb-1">
                            Desired Profile
                          </label>
                          <select
                            value={item.quality_profile_id ?? ''}
                            onChange={(e) =>
                              handleChangeLibraryProfile(
                                item.id,
                                e.target.value ? Number(e.target.value) : null
                              )
                            }
                            className="w-full bg-slate-950 border border-slate-700 rounded-lg px-2 py-1 text-xs text-slate-200"
                          >
                            <option value="">None</option>
                            {profiles.map((p) => (
                              <option key={p.id} value={p.id}>
                                {p.name}
                              </option>
                            ))}
                          </select>
                        </div>
                      )}

                      {isTv && (
                        <LibraryTvSeasonsMenu
                          item={item}
                          notify={notify}
                          onRefreshLibrary={fetchLibrary}
                        />
                      )}
                    </div>
                  </div>

                  <div className="p-3 pt-0">
                    <button
                      type="button"
                      onClick={() =>
                        handleSearchReleases(item.title, item.year, item.id)
                      }
                      className="w-full bg-slate-800 hover:bg-slate-700 text-xs py-1.5 rounded-lg"
                    >
                      Manual Search
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}

        {/* ---------------- DOWNLOADS ---------------- */}
        {tab === 'downloads' && (
          <div className="max-w-4xl mx-auto space-y-3">
            {downloads.length === 0 && (
              <div className="text-center py-16 text-slate-500">
                No active torrents in queue.
              </div>
            )}
            {downloads.map((dl) => (
              <div
                key={dl.id}
                className="bg-slate-900 border border-slate-800 p-4 rounded-xl"
              >
                <div className="flex items-center justify-between mb-2">
                  <div className="font-medium text-sm truncate pr-4">
                    {dl.name}
                  </div>
                  <div className="flex items-center gap-4 text-xs text-slate-400 shrink-0">
                    <span>
                      {(dl.download_speed_bps / (1024 * 1024)).toFixed(2)} MB/s
                    </span>
                    <span className="font-semibold text-indigo-400">
                      {dl.progress_pct.toFixed(1)}%
                    </span>
                    <button
                      type="button"
                      onClick={() => handleDeleteDownload(dl.id)}
                      className="hover:text-rose-400"
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                </div>
                <div className="w-full bg-slate-800 h-2 rounded-full overflow-hidden">
                  <div
                    className={`h-full transition-all ${
                      dl.state === 'completed'
                        ? 'bg-emerald-500'
                        : 'bg-indigo-500'
                    }`}
                    style={{ width: `${dl.progress_pct}%` }}
                  />
                </div>
              </div>
            ))}
          </div>
        )}

        {/* ---------------- SETTINGS ---------------- */}
        {tab === 'settings' && (
          <div className="max-w-4xl mx-auto space-y-6">
            <div className="flex flex-wrap gap-2 border-b border-slate-800 pb-4">
              <button
                type="button"
                onClick={() => setSettingsSection('vpn')}
                className={`px-4 py-2 rounded-lg text-sm font-medium flex items-center gap-2 transition ${
                  settingsSection === 'vpn'
                    ? 'bg-indigo-600 text-white'
                    : 'bg-slate-900 text-slate-400 hover:text-white border border-slate-800'
                }`}
              >
                <Shield className="w-4 h-4" /> VPN (WireGuard)
              </button>
              <button
                type="button"
                onClick={() => setSettingsSection('indexers')}
                className={`px-4 py-2 rounded-lg text-sm font-medium flex items-center gap-2 transition ${
                  settingsSection === 'indexers'
                    ? 'bg-indigo-600 text-white'
                    : 'bg-slate-900 text-slate-400 hover:text-white border border-slate-800'
                }`}
              >
                <Server className="w-4 h-4" /> Indexers
              </button>
              <button
                type="button"
                onClick={() => setSettingsSection('profiles')}
                className={`px-4 py-2 rounded-lg text-sm font-medium flex items-center gap-2 transition ${
                  settingsSection === 'profiles'
                    ? 'bg-indigo-600 text-white'
                    : 'bg-slate-900 text-slate-400 hover:text-white border border-slate-800'
                }`}
              >
                <Sliders className="w-4 h-4" /> Quality Profiles
              </button>
              <button
                type="button"
                onClick={() => setSettingsSection('tmdb')}
                className={`px-4 py-2 rounded-lg text-sm font-medium flex items-center gap-2 transition ${
                  settingsSection === 'tmdb'
                    ? 'bg-indigo-600 text-white'
                    : 'bg-slate-900 text-slate-400 hover:text-white border border-slate-800'
                }`}
              >
                <KeyRound className="w-4 h-4" /> TMDB API
              </button>
            </div>

            {/* Settings -> VPN */}
            {settingsSection === 'vpn' && (
              <form
                onSubmit={handleSaveVpn}
                className="bg-slate-900 border border-slate-800 p-6 rounded-xl space-y-4"
              >
                <div className="flex items-center justify-between">
                  <h2 className="text-lg font-bold flex items-center gap-2">
                    <Shield className="w-5 h-5 text-indigo-400" /> Userspace WireGuard
                    Tunnel & Kill Switch
                  </h2>
                  <label className="flex items-center gap-2 text-sm cursor-pointer">
                    <input
                      type="checkbox"
                      checked={vpnEnabled}
                      onChange={(e) => setVpnEnabled(e.target.checked)}
                      className="rounded accent-indigo-600"
                    />
                    Enable Kill Switch
                  </label>
                </div>

                <p className="text-xs text-slate-400">
                  Upload or paste a standard WireGuard <code>.conf</code> file
                  (Mullvad, ProtonVPN, NordVPN, IVPN). Only BitTorrent traffic is
                  routed through <code>wg-hook</code>.
                </p>

                <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                  <div>
                    <label className="block text-xs text-slate-400 mb-1">
                      Provider Label
                    </label>
                    <input
                      type="text"
                      value={vpnProvider}
                      onChange={(e) => setVpnProvider(e.target.value)}
                      className="w-full bg-slate-950 border border-slate-700 rounded-lg px-3 py-2 text-sm"
                    />
                  </div>

                  <div>
                    <label className="block text-xs text-slate-400 mb-1">
                      Upload .conf File
                    </label>
                    <label className="flex items-center justify-center gap-2 w-full bg-slate-950 hover:bg-slate-800 border border-dashed border-slate-700 rounded-lg px-3 py-2 text-xs text-slate-300 cursor-pointer transition">
                      <Upload className="w-4 h-4 text-indigo-400" />
                      <span>Choose WireGuard .conf...</span>
                      <input
                        type="file"
                        accept=".conf,text/plain"
                        onChange={handleWgFileUpload}
                        className="hidden"
                      />
                    </label>
                  </div>
                </div>

                <div>
                  <label className="block text-xs text-slate-400 mb-1">
                    WireGuard Configuration (.conf)
                  </label>
                  <textarea
                    rows={8}
                    value={wgConfig}
                    onChange={(e) => setWgConfig(e.target.value)}
                    placeholder={`[Interface]\nPrivateKey = ...\nAddress = 10.5.0.2/32\n\n[Peer]\nPublicKey = ...\nEndpoint = 185.213.154.66:51820`}
                    className="w-full bg-slate-950 border border-slate-700 rounded-lg p-3 text-xs font-mono"
                  />
                </div>

                <button
                  type="submit"
                  className="bg-indigo-600 hover:bg-indigo-500 px-5 py-2.5 rounded-lg text-sm font-semibold"
                >
                  Save & Apply Tunnel
                </button>
              </form>
            )}

            {/* Settings -> Indexers */}
            {settingsSection === 'indexers' && (
              <div className="space-y-6">
                <form
                  onSubmit={handleSaveIndexer}
                  className="bg-slate-900 border border-slate-800 p-5 rounded-xl space-y-4"
                >
                  <div className="flex items-center justify-between">
                    <h3 className="font-bold text-sm">
                      {editingIndexerId !== null
                        ? `Edit Indexer #${editingIndexerId}`
                        : 'Add Indexer'}
                    </h3>
                    {editingIndexerId !== null && (
                      <button
                        type="button"
                        onClick={resetIndexerForm}
                        className="text-xs text-slate-400 hover:text-white"
                      >
                        Cancel Edit
                      </button>
                    )}
                  </div>

                  <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
                    <input
                      type="text"
                      required
                      placeholder="Name (e.g. ApiBay)"
                      value={idxName}
                      onChange={(e) => setIdxName(e.target.value)}
                      className="bg-slate-950 border border-slate-700 rounded-lg px-3 py-2 text-sm"
                    />
                    <input
                      type="url"
                      required
                      placeholder="URL (https://apibay.org)"
                      value={idxUrl}
                      onChange={(e) => setIdxUrl(e.target.value)}
                      className="bg-slate-950 border border-slate-700 rounded-lg px-3 py-2 text-sm"
                    />
                    <select
                      value={idxDriver}
                      onChange={(e) => setIdxDriver(e.target.value)}
                      className="bg-slate-950 border border-slate-700 rounded-lg px-3 py-2 text-sm"
                    >
                      <option value="apibay">Apibay (Movies & TV)</option>
                      <option value="yts">YTS (Movies)</option>
                      <option value="eztv">EZTV (TV Shows)</option>
                    </select>
                    <input
                      type="password"
                      placeholder="API Key (optional)"
                      value={idxApiKey}
                      onChange={(e) => setIdxApiKey(e.target.value)}
                      className="bg-slate-950 border border-slate-700 rounded-lg px-3 py-2 text-sm font-mono"
                    />
                  </div>

                  <div className="flex items-center justify-between pt-1">
                    <label className="flex items-center gap-2 text-xs text-slate-300 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={idxEnabled}
                        onChange={(e) => setIdxEnabled(e.target.checked)}
                        className="rounded accent-indigo-600"
                      />
                      Enabled for searches & Wanted scans
                    </label>

                    <button
                      type="submit"
                      className="bg-indigo-600 hover:bg-indigo-500 px-4 py-2 rounded-lg text-sm font-medium flex items-center justify-center gap-1.5"
                    >
                      <Plus className="w-4 h-4" />
                      {editingIndexerId !== null ? 'Save Changes' : 'Add Indexer'}
                    </button>
                  </div>
                </form>

                <div className="space-y-3">
                  {indexers.map((idx) => (
                    <div
                      key={idx.id}
                      className="bg-slate-900 border border-slate-800 p-4 rounded-xl flex items-center justify-between gap-4"
                    >
                      <div className="flex items-center gap-3 min-w-0">
                        <Server className="w-5 h-5 text-indigo-400 shrink-0" />
                        <div className="min-w-0">
                          <div className="font-semibold text-sm flex items-center gap-2">
                            <span className="truncate">{idx.name}</span>
                            {!idx.enabled && (
                              <span className="text-[11px] px-2 py-0.5 rounded-full bg-slate-800 text-slate-400">
                                Disabled
                              </span>
                            )}
                          </div>
                          <div className="text-xs text-slate-400 truncate">
                            {idx.url} • Driver:{' '}
                            <span className="uppercase text-indigo-400">
                              {idx.driver}
                            </span>
                            {idx.api_key ? ' • API Key Set' : ' • No Key'}
                          </div>
                        </div>
                      </div>

                      <div className="flex items-center gap-1.5 shrink-0">
                        <button
                          type="button"
                          onClick={() => startEditingIndexer(idx)}
                          className="px-2.5 py-1.5 rounded-lg text-xs bg-slate-800 hover:bg-slate-700 text-slate-200"
                        >
                          Edit
                        </button>
                        <button
                          type="button"
                          onClick={() => handleDeleteIndexer(idx.id)}
                          className="text-slate-400 hover:text-rose-400 p-2"
                          title="Delete indexer"
                        >
                          <Trash2 className="w-4 h-4" />
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Settings -> Quality Profiles */}
            {settingsSection === 'profiles' && (
              <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
                <div className="space-y-3">
                  <div className="flex items-center justify-between">
                    <h3 className="text-sm font-bold uppercase tracking-wider text-slate-400">
                      Profiles ({profiles.length})
                    </h3>
                  </div>

                  {profiles.map((p) => (
                    <div
                      key={p.id}
                      className={`p-3.5 rounded-xl border transition flex items-center justify-between gap-2 ${
                        editingProfileId === p.id
                          ? 'bg-indigo-950/30 border-indigo-500'
                          : 'bg-slate-900 border-slate-800'
                      }`}
                    >
                      <button
                        type="button"
                        onClick={() => startEditingProfile(p)}
                        className="text-left flex-1 min-w-0"
                      >
                        <div className="font-semibold text-sm truncate">
                          {p.name}
                        </div>
                        <div className="text-xs text-slate-400">
                          Cutoff: {p.cutoff_quality}{' '}
                          {p.upgrade_allowed ? '• Upgrades On' : ''}
                        </div>
                      </button>
                    </div>
                  ))}
                </div>

                <form
                  onSubmit={handleSaveProfile}
                  className="lg:col-span-2 bg-slate-900 border border-slate-800 p-5 rounded-xl space-y-4"
                >
                  <h3 className="font-bold text-base">
                    {editingProfileId === 'new'
                      ? 'Quality Profile'
                      : `Configure ${profileName || `Profile #${editingProfileId}`}`}
                  </h3>

                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                    <div>
                      <label className="block text-xs text-slate-400 mb-1">
                        Profile Name
                      </label>
                      <input
                        type="text"
                        readOnly
                        value={profileName}
                        className="w-full bg-slate-950/70 border border-slate-800 rounded-lg px-3 py-2 text-sm text-slate-300 cursor-not-allowed"
                      />
                    </div>

                    <div>
                      <label className="block text-xs text-slate-400 mb-1">
                        Cutoff Quality
                      </label>
                      <select
                        value={cutoffQuality}
                        onChange={(e) => setCutoffQuality(e.target.value)}
                        className="w-full bg-slate-950 border border-slate-700 rounded-lg px-3 py-2 text-sm"
                      >
                        {availableQualities.map((q) => (
                          <option key={q} value={q}>
                            {q}
                          </option>
                        ))}
                      </select>
                    </div>
                  </div>

                  <label className="flex items-center gap-2 text-sm cursor-pointer">
                    <input
                      type="checkbox"
                      checked={upgradeAllowed}
                      onChange={(e) => setUpgradeAllowed(e.target.checked)}
                      className="rounded accent-indigo-600"
                    />
                    Allow upgrading existing files until cutoff is reached
                  </label>

                  <div className="border border-slate-800 rounded-xl overflow-hidden">
                    <table className="w-full text-left text-xs">
                      <thead className="bg-slate-950 text-slate-400 border-b border-slate-800">
                        <tr>
                          <th className="py-2 px-3">Allowed</th>
                          <th className="py-2 px-3">Quality</th>
                          <th className="py-2 px-3">Rank</th>
                          <th className="py-2 px-3">Max Movie (MB)</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-slate-800">
                        {profileItems.map((item, idx) => (
                          <tr key={item.quality}>
                            <td className="py-2 px-3">
                              <input
                                type="checkbox"
                                checked={item.allowed}
                                onChange={(e) => {
                                  const checked = e.target.checked;
                                  setProfileItems((prev) =>
                                    prev.map((row, i) =>
                                      i === idx ? { ...row, allowed: checked } : row
                                    )
                                  );
                                }}
                                className="rounded accent-indigo-600"
                              />
                            </td>
                            <td className="py-2 px-3 font-medium">
                              {item.quality}
                            </td>
                            <td className="py-2 px-3">
                              <input
                                type="number"
                                value={item.rank}
                                onChange={(e) => {
                                  const rank = Number(e.target.value) || 0;
                                  setProfileItems((prev) =>
                                    prev.map((row, i) =>
                                      i === idx ? { ...row, rank } : row
                                    )
                                  );
                                }}
                                className="w-16 bg-slate-950 border border-slate-700 rounded px-2 py-1"
                              />
                            </td>
                            <td className="py-2 px-3">
                              <input
                                type="number"
                                placeholder="Any"
                                value={item.max_movie_size_mb ?? ''}
                                onChange={(e) => {
                                  const val = e.target.value
                                    ? Number(e.target.value)
                                    : null;
                                  setProfileItems((prev) =>
                                    prev.map((row, i) =>
                                      i === idx
                                        ? { ...row, max_movie_size_mb: val }
                                        : row
                                    )
                                  );
                                }}
                                className="w-24 bg-slate-950 border border-slate-700 rounded px-2 py-1"
                              />
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>

                  <div className="flex justify-end gap-2">
                    {editingProfileId !== 'new' && (
                      <button
                        type="button"
                        onClick={() => resetProfileForm()}
                        className="px-4 py-2 rounded-lg text-sm bg-slate-800 hover:bg-slate-700"
                      >
                        Cancel Edit
                      </button>
                    )}
                    <button
                      type="submit"
                      className="bg-indigo-600 hover:bg-indigo-500 px-5 py-2 rounded-lg text-sm font-semibold"
                    >
                      {editingProfileId === 'new'
                        ? 'Create Profile'
                        : 'Save Changes'}
                    </button>
                  </div>
                </form>
              </div>
            )}

            {/* Settings -> TMDB API Key */}
            {settingsSection === 'tmdb' && (
              <form
                onSubmit={handleSaveTmdbKey}
                className="bg-slate-900 border border-slate-800 p-6 rounded-xl space-y-4 max-w-2xl"
              >
                <div className="flex items-center justify-between">
                  <h2 className="text-lg font-bold flex items-center gap-2">
                    <KeyRound className="w-5 h-5 text-indigo-400" /> TMDB API Key
                  </h2>
                  {tmdbStatus && (
                    <span
                      className={`text-xs px-2.5 py-1 rounded-full border font-semibold ${
                        tmdbStatus.configured
                          ? 'bg-emerald-950/60 border-emerald-700 text-emerald-400'
                          : 'bg-amber-950/60 border-amber-700 text-amber-400'
                      }`}
                    >
                      {tmdbStatus.configured
                        ? `Active (${tmdbStatus.masked_key})`
                        : 'Not Configured'}
                    </span>
                  )}
                </div>

                <p className="text-xs text-slate-400">
                  Enter your TMDB API v3 key or v4 Read Access Token. Saved keys
                  are stored in SQLite and applied immediately without restarting
                  Hook. Leave blank and save to revert to the{' '}
                  <code>TMDB_API_KEY</code> environment variable.
                </p>

                <div>
                  <label className="block text-xs text-slate-400 mb-1">
                    Custom TMDB API Key / Read Token
                  </label>
                  <input
                    type="password"
                    value={tmdbApiKeyInput}
                    onChange={(e) => setTmdbApiKeyInput(e.target.value)}
                    placeholder={
                      tmdbStatus?.masked_key
                        ? `Saved: ${tmdbStatus.masked_key} (paste new key to replace)`
                        : 'Paste your TMDB API key...'
                    }
                    className="w-full bg-slate-950 border border-slate-700 rounded-lg px-3 py-2 text-sm font-mono"
                  />
                </div>

                <button
                  type="submit"
                  disabled={savingTmdb}
                  className="bg-indigo-600 hover:bg-indigo-500 px-5 py-2.5 rounded-lg text-sm font-semibold disabled:opacity-60"
                >
                  {savingTmdb ? 'Saving...' : 'Save TMDB Key'}
                </button>
              </form>
            )}
          </div>
        )}
      </main>

      <ReleaseModal
        title={releaseModalTitle}
        releases={releases}
        searching={searchingReleases}
        error={releaseSearchError}
        availableQualities={availableQualities}
        onClose={() => {
          setReleaseModalTitle(null);
          setActiveReleaseMediaId(null);
          setSearchingReleases(false);
          setReleaseSearchError(null);
        }}
        onGrab={handleGrabRelease}
      />
    </div>
  );
}