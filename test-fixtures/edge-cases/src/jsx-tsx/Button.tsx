interface ButtonProps {
  label: string;
  onClick: () => void;
  variant?: "primary" | "secondary";
}

export function Button({ label, onClick, variant = "primary" }: ButtonProps) {
  return <button className={`btn btn-${variant}`} onClick={onClick}>{label}</button>;
}

export function IconButton({ label, onClick }: ButtonProps) {
  return <button onClick={onClick}><span className="icon" />{label}</button>;
}
