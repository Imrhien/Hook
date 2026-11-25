import React, { useEffect, useState } from 'react';
import {
  Search, Film, Tv, Download, Shield, ShieldAlert, Server,
  FolderSearch, Plus, Trash2, RefreshCw
} from 'lucide-react';

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
}

interface Indexer {
  id: number;
  name: string;
  url: string;
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

type Tab = 'discover' | 'library' | 'indexers' | 'downloads' | 'vpn';

export default function App() {
  const [tab, setTab] = useState<Tab>('discover');
  const [vpn, setVpn] = useState<VpnStatus | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  const [query, setQuery] = useState('');
  const [searchResults, setSearchResults] = useState<TmdbResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [releases, setReleases] = useState<Release[]>([]);
  const [releaseModalTitle, setReleaseModalTitle] = useState<string | null>(null);

  const [library, setLibrary] = useState<LibraryItem[]>([]);
  const [scanPath, setScanPath] = useState('/home/steve/rust/testfiles');
  const [scanning, setScanning] = useState(false);

  const [indexers, setIndexers] = useState<Indexer[]>([]);
  const [idxName, setIdxName] = useState('');
  const [idxUrl, setIdxUrl] = useState('');
  const [idxProvider, setIdxProvider] = useState('apibay');

  const [downloads, setDownloads] = useState<DownloadItem[]>([]);

  const [vpnEnabled, setVpnEnabled] = useState(true);
  const [vpnProvider, setVpnProvider] = useState('NordVPN');
  const [wgConfig, setWgConfig] = useState('');

  const notify = (msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 4000);
  };

  const fetchVpn = async () => {
    try {
      const res = await fetch('/api/vpn');
      if (res.ok) setVpn(await res.json());
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

  const fetchDownloads = async () => {
    const res = await fetch('/api/downloads');
    if (res.ok) setDownloads(await res.json());
  };

  useEffect(() => {
    fetchVpn();
    fetchLibrary();
    fetchIndexers();
    fetchDownloads();
    const interval = setInterval(() => {
      fetchDownloads();
      fetchVpn();
    }, 2500);
    return () => clearInterval(interval);
  }, []);

  const handleSearch = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!query.trim()) return;
    setSearching(true);
    const res = await fetch(`/api/search?q=${encodeURIComponent(query)}`);
    if (res.ok) setSearchResults(await res.json());
    setSearching(false);
  };

  const handleRequestMedia = async (item: TmdbResult) => {
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
      }),
    });
    if (res.ok) {
      const data = await res.json();
      notify(`Requested "${item.title}" (${data.status})`);
      fetchLibrary();
    }
  };

  const handleSearchReleases = async (title: string, year?: number) => {
    const q = year ? `${title} ${year}` : title;
    setReleaseModalTitle(q);
    setReleases([]);
    const res = await fetch(`/api/releases?q=${encodeURIComponent(q)}`);
    if (res.ok) setReleases(await res.json());
  };

  const handleGrabRelease = async (rel: Release) => {
    const res = await fetch('/api/downloads', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        title: rel.title,
        download_url: rel.download_url,
      }),
    });
    if (res.ok) {
      notify(`Started torrent: ${rel.title}`);
      setReleaseModalTitle(null);
      setTab('downloads');
      fetchDownloads();
    } else {
      const err = await res.text();
      notify(`Error: ${err}`);
    }
  };

  const handleScanFolder = async (e: React.FormEvent) => {
    e.preventDefault();
    setScanning(true);
    const res = await fetch('/api/scan', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: scanPath }),
    });
    if (res.ok) {
      const summary = await res.json();
      notify(`Scanned ${summary.files_found} files — imported ${summary.imported_or_updated}`);
      fetchLibrary();
    }
    setScanning(false);
  };

  const handleAddIndexer = async (e: React.FormEvent) => {
    e.preventDefault();
    const res = await fetch('/api/indexers', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: idxName, url: idxUrl, provider: idxProvider }),
    });
    if (res.ok) {
      setIdxName('');
      setIdxUrl('');
      fetchIndexers();
      notify('Indexer added');
    }
  };

  const handleDeleteIndexer = async (id: number) => {
    await fetch(`/api/indexers/${id}`, { method: 'DELETE' });
    fetchIndexers();
  };

  const handleDeleteDownload = async (id: string) => {
    await fetch(`/api/downloads/${id}`, { method: 'DELETE' });
    fetchDownloads();
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

  const formatBytes = (bytes: number) => {
    if (!bytes) return '0 B';
    const gb = bytes / (1024 ** 3);
    return gb >= 1 ? `${gb.toFixed(2)} GB` : `${(bytes / (1024 ** 2)).toFixed(0)} MB`;
  };

  return (
    <div className="min-h-screen bg-slate-950 text-slate-100">
      {toast && (
        <div className="fixed bottom-5 right-5 z-50 bg-indigo-600 text-white px-4 py-3 rounded-lg shadow-xl text-sm font-medium">
          {toast}
        </div>
      )}

      <header className="border-b border-slate-800 bg-slate-900/80 backdrop-blur sticky top-0 z-30">
        <div className="max-w-6xl mx-auto px-4 h-16 flex items-center justify-between">
          <div className="flex items-center gap-6">
            <span className="text-xl font-bold tracking-tight text-indigo-400">🪝 Hook</span>
            <nav className="flex gap-1">
              {(['discover', 'library', 'indexers', 'downloads', 'vpn'] as Tab[]).map((t) => (
                <button
                  key={t}
                  onClick={() => setTab(t)}
                  className={`px-3 py-1.5 rounded-md text-sm font-medium capitalize transition ${
                    tab === t
                      ? 'bg-indigo-600 text-white'
                      : 'text-slate-400 hover:text-white hover:bg-slate-800'
                  }`}
                >
                  {t}
                  {t === 'downloads' && downloads.length > 0 && (
                    <span className="ml-1.5 px-1.5 py-0.5 text-xs bg-indigo-500/30 rounded-full">
                      {downloads.length}
                    </span>
                  )}
                </button>
              ))}
            </nav>
          </div>

          <button
            onClick={() => setTab('vpn')}
            className={`flex items-center gap-2 px-3 py-1.5 rounded-full text-xs font-semibold border ${
              vpn?.enabled
                ? vpn.connected
                  ? 'bg-emerald-950/60 border-emerald-700 text-emerald-400'
                  : 'bg-rose-950/60 border-rose-700 text-rose-400'
                : 'bg-slate-800 border-slate-700 text-slate-400'
            }`}
          >
            {vpn?.enabled ? (
              vpn.connected ? <Shield className="w-3.5 h-3.5" /> : <ShieldAlert className="w-3.5 h-3.5" />
            ) : (
              <ShieldAlert className="w-3.5 h-3.5" />
            )}
            {vpn?.enabled
              ? vpn.connected
                ? `VPN: ${vpn.provider_name} (${vpn.interface})`
                : 'VPN DISCONNECTED (KILL SWITCH)'
              : 'VPN Off'}
          </button>
        </div>
      </header>

      <main className="max-w-6xl mx-auto px-4 py-8">
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
              {searchResults.map((item) => (
                <div
                  key={`${item.media_type}-${item.id}`}
                  className="bg-slate-900 border border-slate-800 rounded-xl overflow-hidden flex flex-col justify-between"
                >
                  <div>
                    {item.poster_path ? (
                      <img
                        src={`https://image.tmdb.org/t/p/w342${item.poster_path}`}
                        alt={item.title}
                        className="w-full aspect-[2/3] object-cover"
                      />
                    ) : (
                      <div className="w-full aspect-[2/3] bg-slate-800 flex items-center justify-center text-slate-500 text-xs">
                        No Poster
                      </div>
                    )}
                    <div className="p-3">
                      <div className="flex items-center gap-1.5 text-xs text-indigo-400 uppercase font-semibold mb-1">
                        {item.media_type === 'movie' ? <Film className="w-3.5 h-3.5" /> : <Tv className="w-3.5 h-3.5" />}
                        {item.media_type} {item.year ? `• ${item.year}` : ''}
                      </div>
                      <h3 className="font-semibold text-sm line-clamp-1">{item.title}</h3>
                    </div>
                  </div>
                  <div className="p-3 pt-0 flex flex-col gap-2">
                    <button
                      onClick={() => handleRequestMedia(item)}
                      className="w-full bg-indigo-600 hover:bg-indigo-500 text-xs font-semibold py-2 rounded-lg transition"
                    >
                      + Request
                    </button>
                    <button
                      onClick={() => handleSearchReleases(item.title, item.year)}
                      className="w-full bg-slate-800 hover:bg-slate-700 text-slate-200 text-xs font-medium py-1.5 rounded-lg transition"
                    >
                      Find Torrents
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {tab === 'library' && (
          <div>
            <form onSubmit={handleScanFolder} className="bg-slate-900 border border-slate-800 p-4 rounded-xl flex gap-3 mb-8">
              <FolderSearch className="w-5 h-5 text-indigo-400 self-center ml-1" />
              <input
                type="text"
                value={scanPath}
                onChange={(e) => setScanPath(e.target.value)}
                placeholder="/path/to/media/folder"
                className="flex-1 bg-slate-950 border border-slate-700 rounded-lg px-3.5 py-2 text-sm"
              />
              <button
                type="submit"
                disabled={scanning}
                className="bg-indigo-600 hover:bg-indigo-500 px-5 py-2 rounded-lg text-sm font-medium flex items-center gap-2"
              >
                <RefreshCw className={`w-4 h-4 ${scanning ? 'animate-spin' : ''}`} />
                {scanning ? 'Scanning...' : 'Scan Disk'}
              </button>
            </form>

            <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-5">
              {library.map((item) => (
                <div key={item.id} className="bg-slate-900 border border-slate-800 rounded-xl overflow-hidden">
                  {item.poster_path ? (
                    <img
                      src={`https://image.tmdb.org/t/p/w342${item.poster_path}`}
                      alt={item.title}
                      className="w-full aspect-[2/3] object-cover"
                    />
                  ) : (
                    <div className="w-full aspect-[2/3] bg-slate-800 flex items-center justify-center text-slate-500 text-xs">
                      No Poster
                    </div>
                  )}
                  <div className="p-3">
                    <div className="flex items-center justify-between mb-1">
                      <span className="text-xs text-slate-400 uppercase">{item.media_type}</span>
                      <span
                        className={`text-[11px] px-2 py-0.5 rounded-full font-semibold ${
                          item.status === 'available'
                            ? 'bg-emerald-950 text-emerald-400 border border-emerald-800'
                            : item.status === 'partial'
                            ? 'bg-amber-950 text-amber-400 border border-amber-800'
                            : 'bg-indigo-950 text-indigo-400 border border-indigo-800'
                        }`}
                      >
                        {item.status}
                      </span>
                    </div>
                    <h3 className="font-semibold text-sm line-clamp-1">{item.title}</h3>
                    <button
                      onClick={() => handleSearchReleases(item.title, item.year)}
                      className="mt-2 w-full bg-slate-800 hover:bg-slate-700 text-xs py-1.5 rounded-lg"
                    >
                      Search Releases
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {tab === 'indexers' && (
          <div className="max-w-3xl mx-auto">
            <form onSubmit={handleAddIndexer} className="bg-slate-900 border border-slate-800 p-5 rounded-xl mb-6 grid grid-cols-1 sm:grid-cols-4 gap-3">
              <input
                type="text"
                required
                placeholder="Name (e.g. PirateBay)"
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
                value={idxProvider}
                onChange={(e) => setIdxProvider(e.target.value)}
                className="bg-slate-950 border border-slate-700 rounded-lg px-3 py-2 text-sm"
              >
                <option value="apibay">Apibay (Movies & TV)</option>
                <option value="yts">YTS (Movies)</option>
                <option value="eztv">EZTV (TV Shows)</option>
              </select>
              <button type="submit" className="bg-indigo-600 hover:bg-indigo-500 rounded-lg text-sm font-medium flex items-center justify-center gap-1.5 py-2">
                <Plus className="w-4 h-4" /> Add Indexer
              </button>
            </form>

            <div className="space-y-3">
              {indexers.map((idx) => (
                <div key={idx.id} className="bg-slate-900 border border-slate-800 p-4 rounded-xl flex items-center justify-between">
                  <div className="flex items-center gap-3">
                    <Server className="w-5 h-5 text-indigo-400" />
                    <div>
                      <div className="font-semibold text-sm">{idx.name}</div>
                      <div className="text-xs text-slate-400">
                        {idx.url} • Provider: <span className="uppercase text-indigo-400">{idx.api_key}</span>
                      </div>
                    </div>
                  </div>
                  <button onClick={() => handleDeleteIndexer(idx.id)} className="text-slate-400 hover:text-rose-400 p-2">
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              ))}
            </div>
          </div>
        )}

        {tab === 'downloads' && (
          <div className="max-w-4xl mx-auto space-y-3">
            {downloads.length === 0 && (
              <div className="text-center py-16 text-slate-500">No active torrents in queue.</div>
            )}
            {downloads.map((dl) => (
              <div key={dl.id} className="bg-slate-900 border border-slate-800 p-4 rounded-xl">
                <div className="flex items-center justify-between mb-2">
                  <div className="font-medium text-sm truncate pr-4">{dl.name}</div>
                  <div className="flex items-center gap-4 text-xs text-slate-400 shrink-0">
                    <span>{(dl.download_speed_bps / (1024 * 1024)).toFixed(2)} MB/s</span>
                    <span className="font-semibold text-indigo-400">{dl.progress_pct.toFixed(1)}%</span>
                    <button onClick={() => handleDeleteDownload(dl.id)} className="hover:text-rose-400">
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                </div>
                <div className="w-full bg-slate-800 h-2 rounded-full overflow-hidden">
                  <div
                    className={`h-full transition-all ${dl.state === 'completed' ? 'bg-emerald-500' : 'bg-indigo-500'}`}
                    style={{ width: `${dl.progress_pct}%` }}
                  />
                </div>
              </div>
            ))}
          </div>
        )}

        {tab === 'vpn' && (
          <form onSubmit={handleSaveVpn} className="max-w-2xl mx-auto bg-slate-900 border border-slate-800 p-6 rounded-xl space-y-4">
            <div className="flex items-center justify-between">
              <h2 className="text-lg font-bold flex items-center gap-2">
                <Shield className="w-5 h-5 text-indigo-400" /> Userspace WireGuard Tunnel & Kill Switch
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
              Paste a standard WireGuard <code>.conf</code> from any provider (Mullvad, ProtonVPN, NordVPN, IVPN). Only BitTorrent traffic is routed through <code>wg-hook</code>.
            </p>
            <div>
              <label className="block text-xs text-slate-400 mb-1">Provider Label</label>
              <input
                type="text"
                value={vpnProvider}
                onChange={(e) => setVpnProvider(e.target.value)}
                className="w-full bg-slate-950 border border-slate-700 rounded-lg px-3 py-2 text-sm"
              />
            </div>
            <div>
              <label className="block text-xs text-slate-400 mb-1">WireGuard Configuration (.conf)</label>
              <textarea
                rows={8}
                value={wgConfig}
                onChange={(e) => setWgConfig(e.target.value)}
                placeholder={`[Interface]\nPrivateKey = ...\nAddress = 10.5.0.2/32\n\n[Peer]\nPublicKey = ...\nEndpoint = 185.213.154.66:51820`}
                className="w-full bg-slate-950 border border-slate-700 rounded-lg p-3 text-xs font-mono"
              />
            </div>
            <button type="submit" className="bg-indigo-600 hover:bg-indigo-500 px-5 py-2.5 rounded-lg text-sm font-semibold">
              Save & Apply Tunnel
            </button>
          </form>
        )}
      </main>

      {releaseModalTitle && (
        <div className="fixed inset-0 bg-black/75 z-50 flex items-center justify-center p-4">
          <div className="bg-slate-900 border border-slate-700 rounded-2xl max-w-3xl w-full max-h-[80vh] flex flex-col">
            <div className="p-4 border-b border-slate-800 flex items-center justify-between">
              <h3 className="font-bold text-sm">Torrents for "{releaseModalTitle}"</h3>
              <button onClick={() => setReleaseModalTitle(null)} className="text-slate-400 hover:text-white text-sm">
                Close
              </button>
            </div>
            <div className="p-4 overflow-y-auto space-y-2">
              {releases.length === 0 ? (
                <div className="text-center py-8 text-slate-400 text-sm">Searching indexers...</div>
              ) : (
                releases.map((rel, i) => (
                  <div key={i} className="bg-slate-950 border border-slate-800 p-3 rounded-lg flex items-center justify-between gap-4">
                    <div className="min-w-0">
                      <div className="text-sm font-medium truncate">{rel.title}</div>
                      <div className="text-xs text-slate-400 mt-0.5 flex gap-3">
                        <span className="text-indigo-400 font-semibold">{rel.indexer_name}</span>
                        <span>{rel.quality}</span>
                        <span>{formatBytes(rel.size_bytes)}</span>
                        <span className="text-emerald-400">{rel.seeders} seeders</span>
                      </div>
                    </div>
                    <button
                      onClick={() => handleGrabRelease(rel)}
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
      )}
    </div>
  );
}
