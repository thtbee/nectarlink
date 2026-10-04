// Nectarlink design preview helpers: icons, Material You seeds, theme switcher.
const ICON = {
  home:'<path d="M3 11l9-7 9 7"/><path d="M5 10v10h14V10"/>',
  msg:'<path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z"/>',
  bell:'<path d="M6 8a6 6 0 1 1 12 0c0 7 3 8 3 8H3s3-1 3-8"/><path d="M10 20a2 2 0 0 0 4 0"/>',
  photo:'<rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="9" cy="9" r="2"/><path d="M21 15l-5-5L5 21"/>',
  apps:'<rect x="4" y="4" width="6" height="6" rx="1.5"/><rect x="14" y="4" width="6" height="6" rx="1.5"/><rect x="4" y="14" width="6" height="6" rx="1.5"/><rect x="14" y="14" width="6" height="6" rx="1.5"/>',
  deck:'<rect x="3" y="5" width="18" height="14" rx="3"/><path d="M8 10h.01M12 10h.01M16 10h.01M8 14h8"/>',
  gear:'<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M2 12h3M19 12h3M4.9 19.1L7 17M17 7l2.1-2.1"/>',
  clip:'<rect x="8" y="3" width="8" height="4" rx="1"/><path d="M16 5h2a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h2"/>',
  mirror:'<rect x="2" y="4" width="14" height="10" rx="2"/><rect x="17" y="8" width="5" height="10" rx="1.5"/><path d="M6 18h6"/>',
  send:'<path d="M22 2L11 13M22 2l-7 20-4-9-9-4z"/>',
  ring:'<path d="M6 8a6 6 0 1 1 12 0c0 7 3 8 3 8H3s3-1 3-8"/><path d="M10 20a2 2 0 0 0 4 0"/>',
  search:'<circle cx="11" cy="11" r="7"/><path d="M20 20l-3.5-3.5"/>',
  play:'<path d="M8 5l11 7-11 7z"/>', pause:'<path d="M8 5v14M16 5v14"/>',
  prev:'<path d="M19 20L9 12l10-8zM5 19V5"/>', next:'<path d="M5 4l10 8-10 8zM19 5v14"/>',
  check:'<path d="M5 12.5l4.5 4.5L19 7.5"/>', x:'<path d="M6 6l12 12M18 6L6 18"/>',
  chev:'<path d="M9 6l6 6-6 6"/>', back:'<path d="M15 6l-6 6 6 6"/>',
  lock:'<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>',
  hex:'<path d="M12 2.5l8.2 4.75v9.5L12 21.5l-8.2-4.75v-9.5z"/><path d="M9 12h6"/>',
  shield:'<path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z"/>',
  bolt:'<path d="M13 2L4 14h7l-1 8 9-12h-7z"/>', usb:'<path d="M12 2v16M8 6l4-4 4 4M7 12v2a5 5 0 0 0 10 0v-2"/>',
  wifi:'<path d="M5 12.5a10 10 0 0 1 14 0M8.5 16a5 5 0 0 1 7 0M2 9a15 15 0 0 1 20 0"/>',
  qr:'<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><path d="M14 14h3v3M21 14v7h-7"/>',
  attach:'<path d="M21 11l-8.5 8.5a5 5 0 0 1-7-7L14 4a3.5 3.5 0 0 1 5 5l-8.5 8.5a2 2 0 0 1-3-3L15 7"/>',
  emoji:'<circle cx="12" cy="12" r="9"/><path d="M8.5 14.5a4.5 4.5 0 0 0 7 0M9 9.5h.01M15 9.5h.01"/>',
  vol:'<path d="M4 9v6h4l5 4V5L8 9z"/><path d="M17 9a4 4 0 0 1 0 6"/>',
  rotate:'<path d="M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6"/>', camera:'<path d="M3 7h4l2-2h6l2 2h4v12H3z"/><circle cx="12" cy="13" r="3.5"/>',
  kb:'<rect x="2" y="6" width="20" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/>',
  screen:'<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>',
  mic:'<rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v4"/>',
  obs:'<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3"/>', moon:'<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>',
  code:'<path d="M8 8l-5 4 5 4M16 8l5 4-5 4"/>', folder:'<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  more:'<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
  plus:'<path d="M12 5v14M5 12h14"/>', handset:'<rect x="7" y="2" width="10" height="20" rx="2.5"/><path d="M11 18h2"/>', filter:'<path d="M4 5h16l-6 8v6l-4-2v-4z"/>'
};
const i = n => `<svg viewBox="0 0 24 24">${ICON[n]}</svg>`;

const SEEDS = {
  honey:{light:{primary:'#8A5100','on-primary':'#FFFFFF','primary-container':'#FFDDB8','on-primary-container':'#2C1600','secondary-container':'#F3E0CB','on-secondary-container':'#261A0C',surface:'#FFF8F3','surface-low':'#FBF2EB','surface-container':'#F5ECE4','surface-container-high':'#EFE6DE','surface-container-highest':'#E9E1D9','on-surface':'#1F1B16','on-surface-variant':'#51453A',outline:'#837468','outline-variant':'#D5C3B5'},
         dark:{primary:'#FFB95F','on-primary':'#4A2800','primary-container':'#693C00','on-primary-container':'#FFDDB8','secondary-container':'#574330','on-secondary-container':'#F3E0CB',surface:'#17130E','surface-low':'#120E0A','surface-container':'#241F1A','surface-container-high':'#2E2924','surface-container-highest':'#39342E','on-surface':'#EBE1D9','on-surface-variant':'#D5C3B5',outline:'#9E8E81','outline-variant':'#51453A'}},
  sage:{light:{primary:'#3B6939','on-primary':'#FFFFFF','primary-container':'#BCF0B4','on-primary-container':'#002204','secondary-container':'#D6E8CF','on-secondary-container':'#111F0F',surface:'#F7FBF1','surface-low':'#F1F5EB','surface-container':'#EBEFE5','surface-container-high':'#E6E9E0','surface-container-highest':'#E0E4DA','on-surface':'#191D17','on-surface-variant':'#424940',outline:'#72796F','outline-variant':'#C2C8BD'},
        dark:{primary:'#A1D39A','on-primary':'#0A390F','primary-container':'#234F24','on-primary-container':'#BCF0B4','secondary-container':'#3B4B38','on-secondary-container':'#D6E8CF',surface:'#10140F','surface-low':'#0C100B','surface-container':'#1D211B','surface-container-high':'#272B25','surface-container-highest':'#32362F','on-surface':'#E0E4DA','on-surface-variant':'#C2C8BD',outline:'#8C9388','outline-variant':'#424940'}},
  lavender:{light:{primary:'#65558F','on-primary':'#FFFFFF','primary-container':'#E9DDFF','on-primary-container':'#201047','secondary-container':'#E8DEF8','on-secondary-container':'#1D192B',surface:'#FDF7FF','surface-low':'#F7F2FA','surface-container':'#F2ECF4','surface-container-high':'#ECE6EE','surface-container-highest':'#E6E0E9','on-surface':'#1D1B20','on-surface-variant':'#49454E',outline:'#7A757F','outline-variant':'#CAC4CF'},
            dark:{primary:'#CFBDFE','on-primary':'#36275D','primary-container':'#4D3D75','on-primary-container':'#E9DDFF','secondary-container':'#4A4458','on-secondary-container':'#E8DEF8',surface:'#141218','surface-low':'#0F0D13','surface-container':'#211F24','surface-container-high':'#2B292F','surface-container-highest':'#36343A','on-surface':'#E6E0E9','on-surface-variant':'#CAC4CF',outline:'#948F99','outline-variant':'#49454E'}},
  ocean:{light:{primary:'#2F628C','on-primary':'#FFFFFF','primary-container':'#CEE5FF','on-primary-container':'#001D33','secondary-container':'#D5E4F7','on-secondary-container':'#0E1D2A',surface:'#F7F9FF','surface-low':'#F1F4FA','surface-container':'#ECEEF4','surface-container-high':'#E6E8EE','surface-container-highest':'#E0E2E8','on-surface':'#181C20','on-surface-variant':'#42474E',outline:'#72777F','outline-variant':'#C2C7CF'},
         dark:{primary:'#9BCBFB','on-primary':'#003354','primary-container':'#0F4A73','on-primary-container':'#CEE5FF','secondary-container':'#3B4858','on-secondary-container':'#D5E4F7',surface:'#101418','surface-low':'#0B0F13','surface-container':'#1C2024','surface-container-high':'#262A2F','surface-container-highest':'#31353A','on-surface':'#E0E2E8','on-surface-variant':'#C2C7CF',outline:'#8C9199','outline-variant':'#42474E'}}
};

// Deterministic fake QR code (finder patterns + pseudo-random modules). Not scannable.
function qrSVG(size = 25, seed = 7) {
  let s = seed, r = () => (s = (s * 9301 + 49297) % 233280) / 233280;
  const finder = (x, y) => `<rect x="${x}" y="${y}" width="7" height="7" fill="currentColor"/><rect x="${x+1}" y="${y+1}" width="5" height="5" fill="var(--qr-bg)"/><rect x="${x+2}" y="${y+2}" width="3" height="3" fill="currentColor"/>`;
  const inFinder = (x, y) => (x < 8 && y < 8) || (x > size - 9 && y < 8) || (x < 8 && y > size - 9);
  let m = '';
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) if (!inFinder(x, y) && r() > 0.52) m += `<rect x="${x}" y="${y}" width="1" height="1"/>`;
  return `<svg viewBox="-1 -1 ${size + 2} ${size + 2}" style="width:100%;height:100%;stroke:none;fill:currentColor">${finder(0,0)}${finder(size-7,0)}${finder(0,size-7)}${m}</svg>`;
}

function setupThemeBar() {
  const body = document.body, themes = document.getElementById('themes'), seeds = document.getElementById('seeds');
  let theme = 'bloom-light', seed = 'honey';
  const apply = () => {
    body.dataset.theme = theme; body.style.cssText = '';
    const bloom = theme.startsWith('bloom');
    if (bloom) { const p = SEEDS[seed][theme.endsWith('dark') ? 'dark' : 'light']; for (const k in p) body.style.setProperty('--' + k, p[k]); }
    body.style.setProperty('--qr-bg', getComputedStyle(body).getPropertyValue('--surface'));
    seeds.classList.toggle('off', !bloom);
  };
  themes.onclick = e => { const b = e.target.closest('button'); if (!b) return; theme = b.dataset.t; [...themes.children].forEach(x => x.classList.toggle('on', x === b)); apply(); };
  seeds.onclick = e => { const b = e.target.closest('button'); if (!b) return; seed = b.dataset.s; [...seeds.querySelectorAll('button')].forEach(x => x.classList.toggle('on', x === b)); apply(); };
  apply();
}
