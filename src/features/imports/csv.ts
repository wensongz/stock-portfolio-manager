/** Split CSV or TSV while preserving delimiters inside double-quoted fields. */
export function splitCsvLine(line: string): string[] {
  let commas = 0;
  let tabs = 0;
  let inQuotes = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === '"') {
      if (inQuotes && line[i + 1] === '"') i++;
      else inQuotes = !inQuotes;
    } else if (!inQuotes) {
      if (ch === ",") commas++;
      else if (ch === "\t") tabs++;
    }
  }
  const delimiter = tabs > commas ? "\t" : ",";
  const result: string[] = [];
  let current = "";
  inQuotes = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === '"') {
      if (inQuotes && line[i + 1] === '"') {
        current += '"';
        i++;
      } else {
        inQuotes = !inQuotes;
      }
    } else if (ch === delimiter && !inQuotes) {
      result.push(current);
      current = "";
    } else {
      current += ch;
    }
  }
  result.push(current);
  return result;
}

export function parseCsvNumber(value: string | undefined): number {
  return Number.parseFloat((value ?? "").replace(/,/g, "").trim());
}

export function stripBom(text: string): string {
  return text.startsWith("\uFEFF") ? text.slice(1) : text;
}

export async function readFileAsText(file: File, encodings = ["utf-8"]): Promise<string[]> {
  return Promise.all(encodings.map((encoding) => new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = (event) => resolve(event.target?.result as string);
    reader.onerror = reject;
    reader.readAsText(file, encoding);
  })));
}
