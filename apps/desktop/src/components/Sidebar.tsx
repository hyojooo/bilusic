import { useEffect } from 'react';
import { NavLink } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { HomeIcon, SearchIcon, LibraryIcon, SettingsIcon } from './icons';
import { usePlaylists } from '@/stores/playlists';

const nav = [
  { to: '/', key: 'nav.home', icon: HomeIcon, end: true },
  { to: '/search', key: 'nav.search', icon: SearchIcon, end: false },
  { to: '/playlists', key: 'nav.myMusic', icon: LibraryIcon, end: false },
  { to: '/settings', key: 'nav.settings', icon: SettingsIcon, end: false },
];

export default function Sidebar() {
  const { t } = useTranslation();
  const { playlists, loadPlaylists } = usePlaylists();

  useEffect(() => {
    void loadPlaylists();
  }, [loadPlaylists]);

  return (
    <aside className="flex w-52 shrink-0 flex-col border-r border-line bg-surface/60 px-2 py-4">
      <div className="px-2 pb-5">
        <span className="font-brand text-2xl text-accent-500 select-none">
          Bilusic
        </span>
      </div>

      <nav className="flex flex-col gap-0.5 px-1">
        {nav.map(({ to, key, icon: Icon, end }) => (
          <NavLink
            key={to}
            to={to}
            end={end}
            className={({ isActive }) =>
              `nav-item ${isActive ? 'nav-item-active' : ''}`
            }
          >
            <Icon />
            <span className="text-sm font-medium">{t(key)}</span>
          </NavLink>
        ))}
      </nav>

      {/* Playlist quick links */}
      {playlists.length > 0 && (
        <div className="mt-4 flex min-h-0 flex-1 flex-col">
          <p className="px-3 pb-1 text-[11px] uppercase tracking-wide text-muted">
            {t('sidebar.playlists')}
          </p>
          <div className="flex-1 space-y-0.5 overflow-y-auto pr-1">
            {playlists.map((pl) => (
              <NavLink
                key={pl.id}
                to={`/playlist/${pl.id}`}
                className={({ isActive }) =>
                  `nav-item !py-1.5 text-sm ${isActive ? 'nav-item-active' : ''}`
                }
              >
                <LibraryIcon width={16} height={16} />
                <span className="truncate">{pl.name}</span>
              </NavLink>
            ))}
          </div>
        </div>
      )}

      <div className="mt-auto px-2 pt-5 leading-relaxed text-muted">
        <p className="font-brand text-xs tracking-wide">
          <span className="text-accent-500">Bilusic</span> ·{' '}
          {t('sidebar.signature')}
        </p>
      </div>
    </aside>
  );
}
