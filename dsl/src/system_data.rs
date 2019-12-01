use specs::System;

pub trait DataItem<'a, 'b> {
    type View;
}

pub type DataType<'a, S> = <S as System<'a>>::SystemData;

pub type DataView<'a, 'b, T> = <T as DataItem<'a, 'b>>::View;

trait MainView<'a> {
    type ViewAllImmutable;
    type ViewAllWithMut;

    fn view(&'a self) -> Self::ViewAllImmutable;
    fn view_mut(&'a mut self) -> Self::ViewAllWithMut;
}
