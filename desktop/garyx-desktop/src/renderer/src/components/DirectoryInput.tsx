import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useI18n } from '@/i18n';

type DirectoryInputProps = {
  value: string;
  onChange: (next: string) => void;
  id?: string;
  placeholder?: string;
};

export function DirectoryInput({ value, onChange, id, placeholder }: DirectoryInputProps) {
  const { t } = useI18n();

  async function handleBrowse() {
    const picked = await window.garyxDesktop.pickDirectory({
      defaultPath: value || null,
    });
    if (picked) {
      onChange(picked);
    }
  }
  return (
    <div className="flex items-center gap-2">
      <Input
        className="flex-1 rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
        id={id}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        type="text"
        value={value}
      />
      <Button
        className="shrink-0 rounded-[14px] border-[#e7e7e5] bg-white text-[#555] shadow-none hover:bg-[#f4f4f2]"
        onClick={handleBrowse}
        size="sm"
        type="button"
        variant="outline"
      >
        {t('Browse...')}
      </Button>
    </div>
  );
}
