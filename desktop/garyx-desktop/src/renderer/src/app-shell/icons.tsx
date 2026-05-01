import {
  Bot,
  Command,
  Cpu,
  Router,
  Settings2,
  SlidersHorizontal,
  type LucideIcon,
} from 'lucide-react';

import type { DesktopWorkspaceFileEntry } from '@shared/contracts';

import type { SettingsTabId } from '../GatewaySettingsPanel';

function SettingsRailIcon({
  glyph: Glyph,
}: {
  glyph: LucideIcon;
}) {
  return (
    <Glyph
      aria-hidden
      className="icon"
      size={16}
      strokeWidth={1.7}
    />
  );
}

// Monochrome version of the official Model Context Protocol favicon mark.
function McpIcon() {
  return (
    <svg aria-hidden className="icon" fill="none" height="16" viewBox="0 0 180 180" width="16">
      <path
        d="M18 84.8528L85.8822 16.9706C95.2548 7.59798 110.451 7.59798 119.823 16.9706C129.196 26.3431 129.196 41.5391 119.823 50.9117L68.5581 102.177"
        stroke="currentColor"
        strokeLinecap="round"
        strokeWidth="12"
      />
      <path
        d="M69.2652 101.47L119.823 50.9117C129.196 41.5391 144.392 41.5391 153.765 50.9117L154.118 51.2652C163.491 60.6378 163.491 75.8338 154.118 85.2063L92.7248 146.6C89.6006 149.724 89.6006 154.789 92.7248 157.913L105.331 170.52"
        stroke="currentColor"
        strokeLinecap="round"
        strokeWidth="12"
      />
      <path
        d="M102.853 33.9411L52.6482 84.1457C43.2756 93.5183 43.2756 108.714 52.6482 118.087C62.0208 127.459 77.2167 127.459 86.5893 118.087L136.794 67.8822"
        stroke="currentColor"
        strokeLinecap="round"
        strokeWidth="12"
      />
    </svg>
  );
}

const vb = '0 0 20 20';
const sw = { strokeWidth: 1.21 };

export function NewThreadIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(2.5,2.5) scale(0.9375)">
        <path d="M10 0.5H4.5C2.29086 0.5 0.5 2.29086 0.5 4.5V11.5C0.5 13.7091 2.29086 15.5 4.5 15.5H11.5C13.7091 15.5 15.5 13.7091 15.5 11.5V5.5" stroke="currentColor" strokeLinecap="round"/>
      </g>
      <g transform="translate(8.1,2.46)">
        <path d="M8.48775 0.188393C8.73894 -0.062798 9.1462 -0.0627974 9.39739 0.188393C9.64859 0.439584 9.64859 0.846845 9.39739 1.09804L8.69035 1.80508L7.7807 0.895442L8.48775 0.188393Z" fill="currentColor"/>
        <path d="M7.17428 1.50187L8.08392 2.41151L2.03437 8.46106C1.68767 8.80776 1.29466 9.10481 0.866529 9.34375L0.506894 9.54447C0.373812 9.61875 0.207594 9.59563 0.0998259 9.48786C-0.00774042 9.3803 -0.0309915 9.21447 0.0428463 9.08147L0.241457 8.72372C0.480768 8.29266 0.779051 7.89709 1.12768 7.54847L7.17428 1.50187Z" fill="currentColor"/>
      </g>
    </svg>
  );
}

export function NewTabIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(3,3) scale(0.9333)">
        <path d="M0.5 7.5H14.5M7.5 0.5V14.5" stroke="currentColor" strokeLinecap="round"/>
      </g>
    </svg>
  );
}

export function BrowserIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(1.5,3.5) scale(0.9444,0.9286)">
        <path d="M0.5 5.5H17.5M3 13.5C1.61929 13.5 0.5 12.3807 0.5 11V3C0.5 1.61929 1.61929 0.5 3 0.5H15C16.3807 0.5 17.5 1.61929 17.5 3V11C17.5 12.3807 16.3807 13.5 15 13.5H3Z" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
        <circle cx="2.7" cy="2.9" r="0.65" fill="currentColor"/>
        <circle cx="4.9" cy="2.9" r="0.65" fill="currentColor"/>
        <circle cx="7.1" cy="2.9" r="0.65" fill="currentColor"/>
      </g>
    </svg>
  );
}

export function PanelIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(1.5,3.5) scale(0.9444,0.9286)">
        <path d="M10.5 0.500001L10.5 13.5M10.5 0.500001L3 0.5C1.61929 0.5 0.500001 1.61929 0.500001 3L0.5 11C0.5 12.3807 1.61929 13.5 3 13.5L10.5 13.5M10.5 0.500001L15 0.500001C16.3807 0.500001 17.5 1.61929 17.5 3V11C17.5 12.3807 16.3807 13.5 15 13.5H10.5M15.5 5.5H12.5M15.5 3.5L12.5 3.5" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function GatewayIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(2.43,1.5) scale(0.938,0.944)">
        <path d="M0.5 8.42897L7.68445 1.24388C8.16089 0.767522 8.80706 0.49994 9.48079 0.5C10.1545 0.50006 10.8006 0.767757 11.277 1.2442C11.7534 1.72064 12.0209 2.36681 12.0209 3.04054C12.0208 3.71427 11.7531 4.36039 11.2767 4.83675M11.2767 4.83675L5.85022 10.2632M11.2767 4.83675L5.92582 10.1889M11.2767 4.83675C11.5126 4.6008 11.7926 4.41428 12.1009 4.28658C12.4091 4.15888 12.7395 4.09316 13.0731 4.09316C13.4067 4.09316 13.7371 4.15888 14.0454 4.28658C14.3536 4.41428 14.6336 4.60144 14.8695 4.83739L14.9067 4.87455C15.1426 5.11044 15.3298 5.3905 15.4575 5.69873C15.5852 6.00697 15.6509 6.33734 15.6509 6.67098C15.6509 7.00462 15.5852 7.33499 15.4575 7.64322C15.3298 7.95146 15.1426 8.23152 14.9067 8.46741L8.40841 14.9657C8.32966 15.0443 8.26719 15.1376 8.22456 15.2403C8.18193 15.343 8.15999 15.4532 8.15999 15.5644C8.15999 15.6756 8.18193 15.7858 8.22456 15.8885C8.26719 15.9912 8.32966 16.0846 8.40841 16.1631L9.74292 17.4976M9.48089 3.0413L4.16719 8.35564C3.69968 8.83383 3.43959 9.47708 3.44336 10.1458C3.44713 10.8146 3.71446 11.4548 4.18734 11.9277C4.66022 12.4006 5.3005 12.6679 5.96924 12.6717C6.63798 12.6755 7.28123 12.4154 7.75942 11.9479L13.0731 6.63417" stroke="currentColor" strokeLinecap="round"/>
      </g>
    </svg>
  );
}

export function ChannelsIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(3.5,3) scale(0.929,0.933)">
        <path d="M1.5 4.50006H13.5M10.5 0.500064L8.5 14.5001M0.5 10.5001H12.5M5.5 0.500064L3.5 14.5001" stroke="currentColor" strokeLinecap="round"/>
      </g>
    </svg>
  );
}

export function SettingsIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(2.28,1.5) scale(0.939,0.944)">
        <path fillRule="evenodd" clipRule="evenodd" d="M10.2397 1.71216C9.9245 0.976794 9.20142 0.5 8.40136 0.5H8.03895C7.23889 0.5 6.51581 0.976795 6.20066 1.71216L5.67299 2.94338C5.40582 3.56678 4.75357 3.93334 4.08214 3.83743L3.03476 3.6878C2.23816 3.574 1.45094 3.94869 1.03693 4.63871L0.785016 5.05857C0.433204 5.64492 0.405342 6.37037 0.711147 6.98198L1.38475 8.32918C1.59589 8.75147 1.59589 9.24853 1.38475 9.67082L0.711148 11.018C0.405343 11.6296 0.433204 12.3551 0.785016 12.9414L1.13755 13.529C1.499 14.1314 2.15001 14.5 2.85254 14.5H3.91738C4.41891 14.5 4.88726 14.7507 5.16546 15.1679L6.12642 16.6094C6.49735 17.1658 7.12182 17.5 7.79052 17.5H8.64979C9.31849 17.5 9.94296 17.1658 10.3139 16.6094L11.2749 15.1679C11.5531 14.7507 12.0214 14.5 12.5229 14.5H13.5878C14.2903 14.5 14.9413 14.1314 15.3028 13.529L15.6553 12.9414C16.0071 12.3551 16.035 11.6296 15.7292 11.018L15.0556 9.67082C14.8444 9.24853 14.8444 8.75147 15.0556 8.32918L15.7292 6.98198C16.035 6.37037 16.0071 5.64492 15.6553 5.05857L15.4034 4.63871C14.9894 3.94869 14.2022 3.574 13.4056 3.6878L12.3582 3.83743C11.6867 3.93334 11.0345 3.56678 10.7673 2.94338L10.2397 1.71216Z" stroke="currentColor"/>
        <path d="M11.2202 9C11.2202 10.6569 9.87701 12 8.22016 12C6.5633 12 5.22016 10.6569 5.22016 9C5.22016 7.34315 6.5633 6 8.22016 6C9.87701 6 11.2202 7.34315 11.2202 9Z" stroke="currentColor"/>
      </g>
    </svg>
  );
}

export function AutomationIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(2,3) scale(0.944,0.939)">
        <path d="M16.5 6.5V4.5C16.5 3.11929 15.3807 2 14 2H3C1.61929 2 0.5 3.11929 0.5 4.5V12.5C0.5 13.8807 1.61929 15 3 15H7.5M0.5 6.5H9M5 0.5V4M12 0.5V4M13 9V10.882C13 11.2607 13.214 11.607 13.5528 11.7764L15 12.5M13 16C10.5147 16 8.5 13.9853 8.5 11.5C8.5 9.01472 10.5147 7 13 7C15.4853 7 17.5 9.01472 17.5 11.5C17.5 13.9853 15.4853 16 13 16Z" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function AutoResearchIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(2.38,2.38) scale(0.938)">
        <path d="M11.9753 6.38162C11.6501 6.01483 11.3067 5.64985 10.9462 5.28936C7.04097 1.38411 2.60881 -0.515381 1.04672 1.04672C0.0593048 2.03413 0.455003 4.1683 1.86612 6.59128M4.26031 9.85394C4.58547 10.2207 4.92887 10.5857 5.28936 10.9462C9.1946 14.8515 13.6268 16.7509 15.1889 15.1889C16.1763 14.2014 15.7806 12.0673 14.3695 9.64429M9.85394 11.9753C10.2207 11.6501 10.5857 11.3067 10.9462 10.9462C14.8515 7.04097 16.7509 2.60881 15.1889 1.04672C14.2014 0.0593033 12.0673 0.455002 9.64429 1.86612M6.38163 4.2603C6.01483 4.58547 5.64985 4.92887 5.28936 5.28936C1.38411 9.1946 -0.515381 13.6268 1.04672 15.1889C2.03413 16.1763 4.1683 15.7806 6.59127 14.3694" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
      <circle cx="10" cy="10" r="1.2" fill="currentColor"/>
    </svg>
  );
}

export function AgentsIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(2.4,2.4) scale(0.96,0.9933)">
        <path d="M8.5 8.73053C7.87444 8.583 7.20127 8.5 6.5 8.5C3.18629 8.5 0.5 10.3532 0.5 12.1118C0.5 13.8704 3.18629 14.5 6.5 14.5C7.01795 14.5 7.52057 14.4846 8 14.4519M6.5 6.5C4.84315 6.5 3.5 5.15685 3.5 3.5C3.5 1.84315 4.84315 0.5 6.5 0.5C8.15685 0.5 9.5 1.84315 9.5 3.5C9.5 5.15685 8.15685 6.5 6.5 6.5Z" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
      <g transform="translate(10.4,10.4)">
        <path d="M0 3.5C0 3.67258 0.110917 3.77112 0.271125 3.80813C2.61267 4.22711 2.76056 4.23942 3.19189 6.71658C3.21654 6.88905 3.32746 7 3.49999 7C3.67253 7 3.77113 6.88905 3.80809 6.71658C4.22711 4.23942 4.37499 4.22711 6.71655 3.80813C6.87676 3.77112 7 3.67258 7 3.5C7 3.32743 6.87676 3.22888 6.71655 3.19187C4.37499 2.77289 4.22711 2.76058 3.80809 0.283419C3.77113 0.123252 3.67253 0 3.49999 0C3.32746 0 3.21654 0.123252 3.19189 0.283419C2.76056 2.76058 2.61267 2.77289 0.271125 3.19187C0.110917 3.22888 0 3.32742 0 3.5Z" fill="currentColor"/>
      </g>
    </svg>
  );
}

export function TeamsIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(3.5,2.7) scale(0.929,0.935)">
        <path d="M10.5 15H11.5C12.6046 15 13.5 14.1046 13.5 13V5.5C13.5 4.39543 12.6046 3.5 11.5 3.5H10.5M10.5 15V3.5M10.5 15H7M10.5 3.5V2.5C10.5 1.39543 9.60457 0.5 8.5 0.5H2.5C1.39543 0.5 0.5 1.39543 0.5 2.5V13C0.5 14.1046 1.39543 15 2.5 15H4M4 15V12C4 11.1716 4.67157 10.5 5.5 10.5C6.32843 10.5 7 11.1716 7 12V15M4 15H7M3.5 7.5H7.5M3.5 4.5H7.5" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function SkillsIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(1.87,0.97) scale(1.065)">
        <path d="M0.684437 8.85732L8.20607 0.723955C8.66243 0.230474 9.4825 0.62096 9.38708 1.28631L8.67188 6.27304C8.61268 6.68584 8.93295 7.05531 9.34997 7.05531H14.0727C14.6658 7.05531 14.9786 7.75778 14.5819 8.19861L7.26383 16.3292C6.80466 16.8394 5.96397 16.4323 6.07941 15.7557L6.92369 10.8077C6.99506 10.3894 6.67279 10.0074 6.24842 10.0074H1.18736C0.589891 10.0074 0.278778 9.29597 0.684437 8.85732Z" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function MemoryIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(1.5,1.5) scale(0.944)">
        <path d="M7.5 0.5V2.5M10.5 0.5V2.5M10.5 15.5V17.5M7.5 15.5V17.5M2.5 7.5H0.5M2.50019 10.5H0.500185M17.5 7.5H15.5M17.5 10.5H15.5M6.5 15.5H11.5C13.7091 15.5 15.5 13.7091 15.5 11.5V6.5C15.5 4.29086 13.7091 2.5 11.5 2.5H6.5C4.29086 2.5 2.5 4.29086 2.5 6.5V11.5C2.5 13.7091 4.29086 15.5 6.5 15.5ZM7.49981 12.5H10.4998C11.6044 12.5 12.4998 11.6046 12.4998 10.5V7.5C12.4998 6.39543 11.6044 5.5 10.4998 5.5H7.49981C6.39525 5.5 5.49981 6.39543 5.49981 7.5V10.5C5.49981 11.6046 6.39525 12.5 7.49981 12.5Z" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function BackIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(6.41,4) scale(0.848,0.923)">
        <path d="M6.0858 12.5001L0.792894 7.20721C0.402369 6.81668 0.402369 6.18351 0.792894 5.79299L6.0859 0.5" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function ForwardIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(8,4) scale(0.848,0.923)">
        <path d="M0.500106 0.5L5.79301 5.7929C6.18353 6.18343 6.18353 6.81659 5.79301 7.20712L0.5 12.5001" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function ExternalLinkIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(2.5,2.08) scale(0.938,0.939)">
        <path d="M15.5 13.8335V14.3335C15.5 15.4381 14.6046 16.3335 13.5 16.3335H2.5C1.39543 16.3335 0.5 15.4381 0.5 14.3335V13.8335M11.3346 0.5L14.3144 3.47978C14.5097 3.67504 14.5097 3.99162 14.3144 4.18689L11.3346 7.16667M3 11.8335C3 10.4367 3.33921 9.12932 3.98214 8.02397C5.96828 4.60932 8.86018 3.8335 13.2143 3.8335H14" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function SettingsTabIcon({ tabId }: { tabId: SettingsTabId }) {
  switch (tabId) {
    case 'labs':
      return <SettingsRailIcon glyph={SlidersHorizontal} />;
    case 'gateway':
      return <SettingsRailIcon glyph={Router} />;
    case 'provider':
      return <SettingsRailIcon glyph={Cpu} />;
    case 'channels':
      return <SettingsRailIcon glyph={Bot} />;
    case 'commands':
      return <SettingsRailIcon glyph={Command} />;
    case 'mcp':
      return <McpIcon />;
    default:
      return <SettingsRailIcon glyph={Settings2} />;
  }
}

export function isLocalSettingsTab(tabId: SettingsTabId): boolean {
  return tabId === 'provider';
}

export function isGatewayConfigSettingsTab(tabId: SettingsTabId): boolean {
  return tabId === 'gateway' || tabId === 'channels' || tabId === 'labs';
}

export function FolderIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(1.5,3) scale(0.944,0.933)">
        <path d="M9 3.5H14.5C16.1569 3.5 17.5 4.84315 17.5 6.5V11.5C17.5 13.1569 16.1569 14.5 14.5 14.5H3.5C1.84315 14.5 0.5 13.1569 0.5 11.5V3.5M9 3.5L7.59373 1.3906C7.2228 0.834202 6.59834 0.5 5.92963 0.5H3.5C1.84315 0.5 0.5 1.84315 0.5 3.5M9 3.5H0.5" stroke="currentColor" strokeLinecap="round"/>
      </g>
    </svg>
  );
}

export function FolderOpenIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(1.6,4) scale(0.918,0.923)">
        <path d="M13.2187 12.5H3.75006C2.14179 12.5 0.819618 11.2318 0.752664 9.62489L0.502664 3.62489C0.431649 1.92052 1.79421 0.5 3.50006 0.5H4.54403C5.07447 0.5 5.58317 0.710714 5.95825 1.08579L7.07957 2.20711C7.2671 2.39464 7.52146 2.5 7.78667 2.5H13.97C15.0357 2.5 15.7579 3.43558 15.8111 4.5M15.8111 4.5H4.71497C3.33837 4.5 2.13842 5.43689 1.80455 6.77239L1.30455 8.77239C0.831188 10.6658 2.26326 12.5 4.21497 12.5H14.0303C15.4069 12.5 16.6069 11.5631 16.9407 10.2276L17.7514 6.98507C18.067 5.72278 17.1122 4.5 15.8111 4.5Z" stroke="currentColor" strokeLinecap="round"/>
      </g>
    </svg>
  );
}

export function RenameIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(3,2.5) scale(0.929,0.938)">
        <path d="M7.22487 0.5L3.49395 0.50752C1.83946 0.510855 0.5 1.85302 0.5 3.50751V12.5C0.5 14.1569 1.84315 15.5 3.5 15.5H10.5032C12.16 15.5 13.5032 14.1569 13.5032 12.5V6.14607M3.5 12H6.99841M3.5 9.5H10.5" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
      <g transform="translate(9.74,1)">
        <path d="M7.69029 0.188393C7.4391 -0.0627974 7.03184 -0.062798 6.78065 0.188393L6.0736 0.895442L6.98324 1.80508L7.69029 1.09804C7.94148 0.846845 7.94148 0.439584 7.69029 0.188393Z" fill="currentColor"/>
        <path d="M6.37681 2.41151L5.46717 1.50187L1.12768 5.84136C0.779051 6.18999 0.480768 6.58555 0.241457 7.01661L0.0428463 7.37436C-0.0309915 7.50736 -0.00774045 7.67319 0.0998262 7.78076C0.207594 7.88853 0.373812 7.91164 0.506894 7.83737L0.866528 7.63665C1.29466 7.3977 1.68767 7.10065 2.03437 6.75395L6.37681 2.41151Z" fill="currentColor"/>
      </g>
    </svg>
  );
}

export function BrowserBackIcon() {
  return (
    <svg aria-hidden width="18" height="18" viewBox="0 0 24 24" fill="none" className="icon">
      <path d="M15.41 7.41L14 6l-6 6 6 6 1.41-1.41L10.83 12z" fill="currentColor"/>
    </svg>
  );
}

export function BrowserForwardIcon() {
  return (
    <svg aria-hidden width="18" height="18" viewBox="0 0 24 24" fill="none" className="icon">
      <path d="M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z" fill="currentColor"/>
    </svg>
  );
}

export function BrowserRefreshIcon() {
  return (
    <svg aria-hidden width="18" height="18" viewBox="0 0 24 24" fill="none" className="icon">
      <path d="M17.65 6.35A7.958 7.958 0 0012 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08A5.99 5.99 0 0112 18c-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z" fill="currentColor"/>
    </svg>
  );
}

export function BrowserCloseTabIcon() {
  return (
    <svg aria-hidden width="14" height="14" viewBox="0 0 24 24" fill="none" className="icon">
      <path d="M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z" fill="currentColor"/>
    </svg>
  );
}

export function DeleteIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(2.5,1.5) scale(0.938,0.941)">
        <path d="M0.5 3.5H15.5M5.5 3.5V2.88095C5.5 1.56599 6.61929 0.5 8 0.5C9.38071 0.5 10.5 1.56599 10.5 2.88095V3.5M6.25 7V13.5M9.75 7V13.5M2.5 3.5H13.5V14.5C13.5 15.8807 12.3807 17 11 17H5C3.61929 17 2.5 15.8807 2.5 14.5V3.5Z" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function ChevronDownIcon({ size = 16, className = 'icon' }: { size?: number; className?: string }) {
  return (
    <svg aria-hidden width={size} height={size} viewBox={vb} fill="none" className={className} style={sw}>
      <g transform="translate(5.96,7.94)">
        <path d="M7.57107 0.5L4.74264 3.32843C4.35212 3.71895 3.71895 3.71895 3.32843 3.32843L0.5 0.5" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function NewFolderIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon" style={sw}>
      <g transform="translate(1.5,2.25) scale(0.944,0.939)">
        <path d="M9 3.5H14.5C16.1569 3.5 17.5 4.84315 17.5 6.5V9M9 3.5L7.59373 1.3906C7.2228 0.834202 6.59834 0.5 5.92963 0.5H3.5C1.84315 0.5 0.5 1.84315 0.5 3.5M9 3.5H0.5M0.5 3.5V11.5C0.5 13.1569 1.84315 14.5 3.5 14.5H9M17.5 13.0001H14.5M14.5 13.0001H11.5M14.5 13.0001V10.0001M14.5 13.0001V16.0001" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function MoreDotsIcon({ size = 16, className = 'icon' }: { size?: number; className?: string }) {
  return (
    <svg aria-hidden width={size} height={size} viewBox={vb} fill="none" className={className} style={sw}>
      <g transform="translate(3,8.8)">
        <circle cx="7.00005" cy="1.2" r="1.2" fill="currentColor"/>
        <circle cx="1.2" cy="1.2" r="1.2" fill="currentColor"/>
        <circle cx="12.8" cy="1.2" r="1.2" fill="currentColor"/>
      </g>
    </svg>
  );
}

export function CloseIcon() {
  return (
    <svg aria-hidden width="14" height="14" viewBox={vb} fill="none" className="icon icon-tight" style={sw}>
      <g transform="translate(3.5,3.55) scale(0.65)">
        <path d="M13.4549 0.454864L0.457065 13.4527M0.454864 0.454864L13.4527 13.4527" stroke="currentColor" strokeWidth="0.909727" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function DotsIcon() {
  return (
    <svg aria-hidden width="14" height="14" viewBox={vb} fill="none" className="icon icon-tight" style={sw}>
      <g transform="translate(3,8.8)">
        <circle cx="7.00005" cy="1.2" r="1.2" fill="currentColor"/>
        <circle cx="1.2" cy="1.2" r="1.2" fill="currentColor"/>
        <circle cx="12.8" cy="1.2" r="1.2" fill="currentColor"/>
      </g>
    </svg>
  );
}

export function InfoIcon() {
  return (
    <svg aria-hidden width="16" height="16" viewBox={vb} fill="none" className="icon icon-tight" style={sw}>
      <g transform="translate(2,2) scale(0.941)">
        <path d="M7.5 7L8.5 6.5V13.5L9.5 13M16.5 8.5C16.5 12.9183 12.9183 16.5 8.5 16.5C4.08172 16.5 0.5 12.9183 0.5 8.5C0.5 4.08172 4.08172 0.5 8.5 0.5C12.9183 0.5 16.5 4.08172 16.5 8.5Z" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
        <circle cx="8.5" cy="4" r="0.8" fill="currentColor"/>
      </g>
    </svg>
  );
}

export function LockIcon() {
  return (
    <svg aria-hidden width="13" height="13" viewBox={vb} fill="none" className="icon icon-tight" style={sw}>
      <g transform="translate(2.5,2) scale(0.938,0.941)">
        <path d="M3.5 7.5V5C3.5 2.51472 5.51472 0.5 8 0.5C10.4853 0.5 12.5 2.51472 12.5 5V7.5M2.5 16.5H13.5C14.6046 16.5 15.5 15.6046 15.5 14.5V9.5C15.5 8.39543 14.6046 7.5 13.5 7.5H2.5C1.39543 7.5 0.5 8.39543 0.5 9.5V14.5C0.5 15.6046 1.39543 16.5 2.5 16.5ZM8 14C6.89543 14 6 13.1046 6 12C6 10.8954 6.89543 10 8 10C9.10457 10 10 10.8954 10 12C10 13.1046 9.10457 14 8 14Z" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function RefreshIcon() {
  return (
    <svg aria-hidden width="14" height="14" viewBox={vb} fill="none" className="icon icon-tight" style={sw}>
      <g transform="translate(1,3) scale(0.9,0.933)">
        <path d="M16.5161 7.50522C16.5161 11.3741 13.3798 14.5104 9.51091 14.5104C6.89529 14.5104 4.61448 13.0769 3.41129 10.9527M2.50569 7.50522C2.50569 3.63635 5.64203 0.5 9.51091 0.5C12.1265 0.5 14.4073 1.93351 15.6105 4.05773M0.500019 5.50427L2.18522 7.61427C2.35761 7.83012 2.67238 7.86527 2.88814 7.69277L4.99751 6.0063M18.499 9.50534L16.7856 7.36358C16.6234 7.16095 16.3333 7.11602 16.1175 7.26013L13.504 9.00536" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}

export function UploadIcon() {
  return (
    <svg aria-hidden width="14" height="14" viewBox={vb} fill="none" className="icon icon-tight" style={sw}>
      <g transform="translate(2.5,2.75) scale(0.938,0.935)">
        <path d="M3.5 4.66863L7.43432 0.734314C7.74673 0.421895 8.25327 0.421895 8.56569 0.734315L12.5 4.66863M8 0.5V12.002M15.5 12.5021V13.0021C15.5 14.1067 14.6046 15.0021 13.5 15.0021H2.5C1.39543 15.0021 0.5 14.1067 0.5 13.0021V12.5021" stroke="currentColor" strokeLinecap="round"/>
      </g>
    </svg>
  );
}

export function WorkspaceFileIcon({
  entry,
  open = false,
}: {
  entry: Pick<DesktopWorkspaceFileEntry, 'entryType' | 'mediaType'>;
  open?: boolean;
}) {
  if (entry.entryType === 'directory') {
    if (open) {
      return (
        <svg aria-hidden width="15" height="15" viewBox={vb} fill="none" className="icon icon-tight" style={sw}>
          <g transform="translate(1.6,4) scale(0.918,0.923)">
            <path d="M13.2187 12.5H3.75006C2.14179 12.5 0.819618 11.2318 0.752664 9.62489L0.502664 3.62489C0.431649 1.92052 1.79421 0.5 3.50006 0.5H4.54403C5.07447 0.5 5.58317 0.710714 5.95825 1.08579L7.07957 2.20711C7.2671 2.39464 7.52146 2.5 7.78667 2.5H13.97C15.0357 2.5 15.7579 3.43558 15.8111 4.5M15.8111 4.5H4.71497C3.33837 4.5 2.13842 5.43689 1.80455 6.77239L1.30455 8.77239C0.831188 10.6658 2.26326 12.5 4.21497 12.5H14.0303C15.4069 12.5 16.6069 11.5631 16.9407 10.2276L17.7514 6.98507C18.067 5.72278 17.1122 4.5 15.8111 4.5Z" stroke="currentColor" strokeLinecap="round"/>
          </g>
        </svg>
      );
    }
    return (
      <svg aria-hidden width="15" height="15" viewBox={vb} fill="none" className="icon icon-tight" style={sw}>
        <g transform="translate(1.5,3) scale(0.944,0.933)">
          <path d="M9 3.5H14.5C16.1569 3.5 17.5 4.84315 17.5 6.5V11.5C17.5 13.1569 16.1569 14.5 14.5 14.5H3.5C1.84315 14.5 0.5 13.1569 0.5 11.5V3.5M9 3.5L7.59373 1.3906C7.2228 0.834202 6.59834 0.5 5.92963 0.5H3.5C1.84315 0.5 0.5 1.84315 0.5 3.5M9 3.5H0.5" stroke="currentColor" strokeLinecap="round"/>
        </g>
      </svg>
    );
  }
  if (entry.mediaType === 'application/pdf') {
    return (
      <svg aria-hidden width="15" height="15" viewBox={vb} fill="none" className="icon icon-tight" style={sw}>
        <g transform="translate(3.5,2.5) scale(0.929,0.939)">
          <path d="M8.49999 0.738285C8.21141 0.583206 7.88652 0.5 7.55322 0.5H3.89571C1.95123 0.5 0.483808 2.05559 0.500135 4L0.500269 12.5C0.516521 14.4355 2.06449 16.02 4 16H10C11.9188 15.9802 13.5002 14.3208 13.5002 12.4019V6.34396C13.5002 6.04946 13.4352 5.76178 13.3134 5.5M8.49999 0.738285C8.66275 0.825746 8.81395 0.936066 8.94862 1.06722L12.8956 4.91117C13.0712 5.08218 13.212 5.28212 13.3134 5.5M8.49999 0.738285V3.5C8.49999 4.44281 8.49999 4.91421 8.79288 5.20711C9.08578 5.5 9.55718 5.5 10.5 5.5H13.3134" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
        </g>
      </svg>
    );
  }
  if (entry.mediaType === 'text/markdown' || entry.mediaType === 'text/html') {
    return (
      <svg aria-hidden width="15" height="15" viewBox={vb} fill="none" className="icon icon-tight" style={sw}>
        <g transform="translate(3.5,2.5) scale(0.929,0.939)">
          <path d="M3.5 13V8L7 11.5L10.5 8V13M8.49999 0.738285C8.21141 0.583206 7.88652 0.5 7.55322 0.5H3.89571C1.95123 0.5 0.483808 2.05559 0.500135 4L0.500269 12.5C0.516521 14.4355 2.06449 16.02 4 16H10C11.9188 15.9802 13.5002 14.3208 13.5002 12.4019V6.34396C13.5002 6.04946 13.4352 5.76178 13.3134 5.5M8.49999 0.738285C8.66275 0.825746 8.81395 0.936066 8.94862 1.06722L12.8956 4.91117C13.0712 5.08218 13.212 5.28212 13.3134 5.5M8.49999 0.738285V3.5C8.49999 4.44281 8.49999 4.91421 8.79288 5.20711C9.08578 5.5 9.55718 5.5 10.5 5.5H13.3134" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
        </g>
      </svg>
    );
  }
  return (
    <svg aria-hidden width="15" height="15" viewBox={vb} fill="none" className="icon icon-tight" style={sw}>
      <g transform="translate(3.5,2.5) scale(0.929,0.939)">
        <path d="M3.49999 13H10.5M3.49999 10.5H10.5M8.49999 0.738285C8.21141 0.583206 7.88652 0.5 7.55322 0.5H3.89571C1.95123 0.5 0.483808 2.05559 0.500135 4L0.500269 12.5C0.516521 14.4355 2.06449 16.02 4 16H10C11.9188 15.9802 13.5002 14.3208 13.5002 12.4019V6.34396C13.5002 6.04946 13.4352 5.76178 13.3134 5.5M8.49999 0.738285C8.66275 0.825746 8.81395 0.936066 8.94862 1.06722L12.8956 4.91117C13.0712 5.08218 13.212 5.28212 13.3134 5.5M8.49999 0.738285V3.5C8.49999 4.44281 8.49999 4.91421 8.79288 5.20711C9.08578 5.5 9.55718 5.5 10.5 5.5H13.3134" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round"/>
      </g>
    </svg>
  );
}
