import { useEffect, useRef } from "react";
import { X } from "lucide-react";
export default function Modal({
  title,
  close,
  closeLabel,
  children,
}: {
  title: string;
  close: () => void;
  closeLabel: string;
  children: React.ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current!;
    dialog.showModal();
    return () => dialog.close();
  }, []);
  return (
    <dialog
      ref={ref}
      className="modal"
      onCancel={close}
      onClick={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div className="section-heading">
        <h2>{title}</h2>
        <button className="icon-button" aria-label={closeLabel} onClick={close}>
          <X size={19} />
        </button>
      </div>
      {children}
    </dialog>
  );
}
