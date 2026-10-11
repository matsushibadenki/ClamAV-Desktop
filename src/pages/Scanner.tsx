import { ScanChoices, ScanResult, type ViewProps } from "../components/shared";
export default function Scanner(props: ViewProps) {
  const { data, t } = props;
  return (
    <>
      <header className="page-heading">
        <h1>{t("scanTitle")}</h1>
        <p>{t("scanDesc")}</p>
      </header>
      <section className="panel">
        <h2>{t("startScan")}</h2>
        <ScanChoices {...props} />
      </section>
      {data.scan && (
        <div className="panel spaced">
          <ScanResult {...props} scan={data.scan} />
        </div>
      )}
    </>
  );
}
