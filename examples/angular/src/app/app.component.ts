import { Component } from "@angular/core";

@Component({
	selector: "app-root",
	templateUrl: "./app.component.html",
	// uncommenting this freezes rspack
	styleUrls: ["./app.component.scss"]
	// TODO: Figure out why inline styles as template string does not work
})
export class AppComponent {
	title = "rspack-ngs";
}
