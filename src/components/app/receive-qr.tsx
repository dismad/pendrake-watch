import { useEffect, useState } from "react";
import { toast } from "sonner";
import { IconCheck, IconCopy, IconQrcode } from "@tabler/icons-react";
import QRCode from "qrcode";
import { useWalletData } from "@/hooks/use-wallet-data";
import { useMasked } from "@/lib/discreet";
import { splitAddress } from "@/lib/format";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { DiscreetValue } from "@/components/ui/discreet-value";

function formatUa(ua: string): string {
  const { prefix, head, tail } = splitAddress(ua, 8);
  return tail ? `${prefix}${head}…${tail}` : `${prefix}${head}`;
}

export function ReceiveQrButton({ disabled }: { disabled?: boolean }) {
  const { addresses } = useWalletData();
  const masked = useMasked();
  const ua = addresses[0]?.ua ?? "";
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!ua || masked) {
      setDataUrl(null);
      return;
    }
    let cancelled = false;
    QRCode.toDataURL(ua, {
      margin: 1,
      width: 240,
      color: { dark: "#0b1220", light: "#ffffff" },
      errorCorrectionLevel: "M",
    })
      .then((url) => {
        if (!cancelled) setDataUrl(url);
      })
      .catch(() => {
        if (!cancelled) setDataUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, [ua, masked]);

  async function onCopy() {
    if (!ua || masked) return;
    try {
      await navigator.clipboard.writeText(ua);
      setCopied(true);
      toast.success("Address copied");
      window.setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      toast.error(String(e));
    }
  }

  return (
    <Popover>
      <PopoverTrigger
        disabled={disabled || !ua}
        aria-label="Show receive address QR"
        title="Receive"
        className="inline-flex size-7 items-center justify-center rounded-md text-white/55 outline-none hover:bg-white/10 hover:text-white disabled:opacity-40"
      >
        <IconQrcode className="size-4" />
      </PopoverTrigger>
      <PopoverContent
        align="end"
        side="bottom"
        className="w-64 border-white/10 bg-[#12141c] p-3 text-white"
      >
        <p className="mb-2 text-[11px] font-medium uppercase tracking-wide text-white/45">
          Receive
        </p>
        {masked ? (
          <div className="flex aspect-square items-center justify-center rounded-lg bg-white/5 text-xs text-white/50">
            Hidden in discreet mode
          </div>
        ) : dataUrl ? (
          <img
            src={dataUrl}
            alt="Unified address QR"
            className="w-full rounded-lg bg-white p-2"
          />
        ) : (
          <div className="flex aspect-square items-center justify-center rounded-lg bg-white/5 text-xs text-white/50">
            No address yet
          </div>
        )}
        <div className="mt-2 flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-white/70">
            {ua ? (
              <DiscreetValue kind="address">{formatUa(ua)}</DiscreetValue>
            ) : (
              "—"
            )}
          </span>
          <button
            type="button"
            onClick={onCopy}
            disabled={!ua || masked}
            className="inline-flex size-7 items-center justify-center rounded-md text-white/70 hover:bg-white/10 hover:text-white disabled:opacity-40"
            aria-label="Copy unified address"
          >
            {copied ? (
              <IconCheck className="size-3.5" />
            ) : (
              <IconCopy className="size-3.5" />
            )}
          </button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
