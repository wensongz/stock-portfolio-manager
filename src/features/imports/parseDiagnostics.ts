import type { ImportParseIssue } from "./types.ts";

export function parseImportNumber(value: string | undefined): number {
  const cleaned = (value ?? "").replace(/,/g, "").trim();
  if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(cleaned)) return Number.NaN;
  const number = Number(cleaned);
  return Number.isFinite(number) ? number : Number.NaN;
}

export function hasInvalidOptionalImportNumber(value: string | undefined): boolean {
  const cleaned = (value ?? "").trim();
  return !!cleaned && !/^-+$/.test(cleaned) && !Number.isFinite(parseImportNumber(cleaned));
}

export function isImportSummary(value: string): boolean {
  return /^(?:(?:sub\s*)?total|summary)(?:\b|[:：])|^(?:合计|总计|小计|汇总)(?:$|[\s:：])/i.test(value.trim());
}

export function recordImportIssue(issues: ImportParseIssue[] | undefined, line: number, raw: string, messages: string[]): void {
  if (messages.length) issues?.push({ line, raw, message: messages.join("；") });
}

/** Validate calendar dates before date libraries can normalize an invalid day/month. */
export function isValidImportDate(raw: string): boolean {
  const cleaned = raw.trim();
  if (!cleaned) return false;
  const match = cleaned.match(/^(\d{4})[-/](\d{1,2})[-/](\d{1,2})(?=$|[,T\s])/)
    ?? cleaned.match(/^(\d{4})(\d{2})(\d{2})(?=$|[T\s])/);
  if (match) {
    const year = Number(match[1]);
    const month = Number(match[2]);
    const day = Number(match[3]);
    const date = new Date(Date.UTC(year, month - 1, day));
    if (date.getUTCFullYear() !== year || date.getUTCMonth() !== month - 1 || date.getUTCDate() !== day) return false;
    const rest = cleaned.slice(match[0].length).replace(/^[,T\s]+/, "");
    if (!rest) return true;
    const time = rest.match(/^(\d{1,2}):(\d{2})(?::(\d{2})(?:\.\d+)?)?(?:Z|[+-](\d{2}):?(\d{2}))?$/i);
    return !!time && Number(time[1]) < 24 && Number(time[2]) < 60 && Number(time[3] ?? 0) < 60
      && Number(time[4] ?? 0) < 24 && Number(time[5] ?? 0) < 60;
  }
  return Number.isFinite(Date.parse(cleaned));
}
