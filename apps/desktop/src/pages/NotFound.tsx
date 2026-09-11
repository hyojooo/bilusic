import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

export default function NotFound() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  return (
    <div className="flex h-full flex-col items-center justify-center text-center">
      <span className="font-brand text-5xl text-accent-500">404</span>
      <p className="mt-3 text-sm text-muted">{t("notFound.title")}</p>
      <button onClick={() => navigate("/")} className="btn-accent mt-5">
        {t("notFound.back")}
      </button>
    </div>
  );
}
