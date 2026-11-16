export type MenuItem =
  | { separator: true }
  | {
      separator?: false;
      label: string;
      shortcut?: string;
      danger?: boolean;
      disabled?: boolean;
      checked?: boolean;
      color?: string;
      action?: () => void;
      submenu?: MenuItem[] | (() => Promise<MenuItem[]>);
    };

export const menu = $state({ open: false, x: 0, y: 0, items: [] as MenuItem[] });

export function showMenu(e: MouseEvent, items: MenuItem[]) {
  e.preventDefault();
  menu.x = e.clientX;
  menu.y = e.clientY;
  menu.items = items;
  menu.open = true;
}
export function closeMenu() {
  menu.open = false;
}
