import { Info } from "lucide-react";
export default function InfoHint({
  text,
  label,
}: {
  text: string;
  label: string;
}) {
  return (
    <button
      type="button"
      className="icon-button info-hint"
      title={text}
      aria-label={`${label}: ${text}`}
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
      }}
    >
      <Info size={16} />
    </button>
  );
}
